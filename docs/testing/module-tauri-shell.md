# Module: `desktop-tauri` shell — `desktop/src-tauri/src` entry points, process lifecycle, macOS integration

Status: **measured, not at target** — **150** uncovered lines in this unit's scope in a
session that delivers `RunEvent::Ready` (**§S8.3, the pinned ledger**), down from **255** at
the end of Segment 3, **270** at the end of Segment 2 and **400** at `8449acf6`. **Segment 17
(§S13) landed REV-TAURI-SHELL's four families and measured 115 uncovered in the twelve scopes
(both flavours unioned), 88 of the review's 90 fallen rows closed; §S13.10 then closed the three
mock-reachable window rows, leaving **109** uncovered and **87** as the labelled projection for a
`Ready`-delivering session.** The latest measurement (**§S10.4**, 217) was taken in a session that does **not** deliver `Ready`
(§S10.3, §S11.1): **75** of those lines are `host-state-blocked` — covered in §S8.3's artifact,
so **not waivers** — **3** are `needs-window`, and **139** are waived under the ledger's
categories with a named mechanism (§S10.4 carries the corrected arithmetic; §S11.4 records why
5 of the original 80 were re-classified, §S11.7 the forensics behind one of the 139). **0 rows
are open on this unit's own account**, and 9
of the single-instance callback's 12 lines are covered (§S10.5). **Segment 16 (§S12) landed both
outstanding supervisor rulings** — the granted test for `desktop_shutdown.rs` 256–258 and the
driver's fail-loud + one-retry hand-off reporting — and **took no measurement**, because Check A
still answers `without` while Check B (the hand-off, a separate check) passes first-try.

**Revision map — read this before the numbers below.** This record was written over four
measurements, and the sections carry their own revision's line numbers:

| sections | revision measured | residual | authoritative ledger / index |
|---|---|---|---|
| 1–8 | `8449acf6` (campaign baseline), first segment | 318 | §4, §8 (kept for the historical record — Segment 2 and the review unit cite `§4.11` and `§8` rows by number) |
| S2.1–S2.9 | Segment 2 (`/tmp/tauri-shell-lcov-seg2b.info`) | 270 | §S2.6 + its per-line index |
| S3.1–S3.7 | Segment 3 (`/tmp/tauri-shell-lcov-seg3b.info`) | 255 | §S3.4 + its per-line index |
| S4.1–S4.8 | Segment 4 (`/tmp/tauri-shell-lcov-seg4union.info`, per-flavour reports unioned) | 242 | §S4.4 + §S4.5 (the pinned ledger the campaign cites) |
| S5.1–S5.6 | Segment 5 — **no source change**: the Segment-4 measurement re-run | 242 pinned; 245 measured this run (+3 = the two known instabilities) | §S4.4 stays authoritative; §S5.1 has the reproduction arithmetic |
| S6.1–S6.8 | Segment 6 — the auto-connect pass's two remaining arms + a silent-exit test bug fixed | **268** (production **235**; 33 test-region artifact/flaky rows) | **§S6.4 + its per-line index — the current one** |
| S7.1–S7.9 | Segment 7 — the ready arm covered via the peer test support | 238 | §S7.5 + its 238-row index |
| S8.1–S8.6 | Segment 8 — the main-thread driver (route iii, `harness = false` test target) | 150 | §S8.3 + its 150-row index — **the pinned artifact** |
| S9.1–S9.7 | Segment 9 — the single-instance callback driven from a second process; measured 220 (9 target lines covered, 70 lost to driver timing — §S9.3) | 220 | §S9.5 + its 220-row index (superseded by S10) |
| **S10.1–S10.7** | **Segment 10 — the driver reworked so its own coverage no longer depends on the host delivering `Ready` (unconditional notification; a tight, overridable readiness budget); measured 217 in a session where `Ready` is ABSENT (§S10.3), of which 75 are `host-state-blocked` (§S11.4 corrected this from 80) and 9 are this unit's own new coverage** | **217 this session; 150 stays the pinned healthy-session number** | **§S10.4 + its 217-row index** |
| **S11.1–S11.11** | **Segment 11 — no source change: the `Ready`-delivering re-measurement is BLOCKED by the host (`error 125` on `gui/501`; the agent predates the 19:11:51 UTC session restart), five lines of Segment 10's classification corrected, §S11.7 records that the headless leg's `--lib` target is SIGTERM-killed mid-run (chronic: also in Segment 9; it does **not** reproduce standalone) — costing this unit exactly one line — §S11.8 the Segment-12 precondition (`without`, hand-off failing), and §S11.9/§S11.10 the Segment-13 and Segment-14 re-runs of both checks (both still fail: Check A `without` in four observations, Check B — the single-instance hand-off — failing in three, with the listener bind measured at ~25 s in this session; §S11.11 the Segment-15 precondition — Check A still `without` (5th observation), **Check B passing for the first time since Segment 10**, so the hand-off failure was intermittent/load-dependent rather than permanent; mitigation options still unlanded for a decision)** | **217 (same artifact and revision as Segment 10); no measurement taken in Segments 12–15** | **§S10.4 unchanged; §S11.4 carries the corrected class split, §S11.7 the forensics, §S11.8–§S11.11 the preconditions** |
| **S12.1–S12.4** | **Segment 16 — the two supervisor rulings landed: `desktop_shutdown.rs` 256–258 get their granted test in `coordinator.rs` (test-only, no production line touched), and the driver prints `hand-off FAILED` with the computed socket plus one bounded retry. The precondition still answers `without` for Check A (6th observation) while Check B passes first-try (2nd consecutive), so no measurement was taken; the coordinator module’s `a_success_after_the_deadline…` fixture was also found 9/10 red under parallelism and fixed (test-only)** | **217 (artifact unchanged); §S8.3’s 150 stays the pinned healthy-session figure** | **§S10.4 unchanged; §S12 is this segment’s record** |
| **S13.1–S13.9** | **Segment 17 (TAU-SHELL-LAND) — REV-TAURI-SHELL’s four families landed as ordinary tests (`macos_power` observers, the main-thread menu builder, `tests/re_pair_entry_points.rs` for the launcher and the bin, the mock loop’s `ExitRequested`), measured on the current source with both flavours unioned: **115 uncovered**. 88 of the review’s 90 FELL rows are now covered; the 2 that resisted are named (`main.rs:51` a brace, `lib.rs:1040` the `setup` hook). 25 of the 115 are `host-state-blocked`, so the labelled projection for a `Ready`-delivering session is **90** — neither 107 nor 127** | **115 measured at that revision; 90 projected (`Ready` absent, 7th observation); §S8.3’s 150 stays the pinned healthy-session figure** | **§S13.5 is that revision’s per-line ledger, §S13.3 the per-family closure, §S13.4 the six mechanism corrections** |
| **S13.10** | **Segment 17b (SHELL-TAIL) — a mock app with a window labelled `main` runs `activate_main_window`'s three activation calls (`lib.rs` 212, 215, 244) and, with them, the `}` of each `if let` body (214, 217, 246); re-measured with the gui leg only on the current source, unioned with the headless leg §S13.2's argument still covers: **109 uncovered**. Classes: 22 `host-state-blocked`, 73 `unreachable-in-this-environment`, 6 + 6 + 2 unchanged** | **109 measured (current source); 87 projected (`Ready` absent, 8th observation)** | **§S13.10 is this measurement and its 109-row index — the pairing's current target** |


A top-to-bottom reader should read **§S8.3 for the pinned numbers**, **§S10.3/§S10.4 for the
latest measurement and its host-state finding**, **§S13 for the current source’s measured figure
and its per-line ledger**, and **§S11.1/§S11.3 for why the pinned number
cannot currently be reproduced by any run in this environment** (Segment 4's 242
stays pinned because the campaign and the review unit cite it; §S6.2 states the line-by-line
relation between the two). §2's "before/after" columns and §4/§8 describe the first revision
and are annotated as such.

**This unit cannot reach zero uncovered lines on a healthy session.** The pinned §S8.3
residual is made of the families §S8.3 and §S10.4 catalogue with a mechanism each: real
Tauri/Wry GUI events (menu clicks, window close requests, sleep/wake/power-off
notifications), the launcher *process* and its `argv` arms, `cfg(not(test))` blocks,
fixture-internal invariants, llvm-cov block-end attribution, private Tauri APIs, and the
Linux-only arm. No test-side work changes that; the remaining seams are §5.2/§5.3 and both
are production-shape changes.

Metric: per-line LCOV `DA:<line>,0` from the **merged `gui` + `headless`** measurement
(§1, and §S4.1 for the per-flavour rendering that supersedes the single-report step),
never the `summary` block.


Owner: `todo_7fe47d4875d4` (unit TAU-C, goal `goal_160bc247aa63`). Write scope:

```
desktop/src-tauri/src/lib.rs            desktop/src-tauri/src/main.rs
desktop/src-tauri/src/menu.rs           desktop/src-tauri/src/launch.rs
desktop/src-tauri/src/instance.rs       desktop/src-tauri/src/runtime.rs
desktop/src-tauri/src/host_environment.rs
desktop/src-tauri/src/macos_power.rs    desktop/src-tauri/src/linux_power.rs
desktop/src-tauri/src/windows_power.rs
desktop/src-tauri/src/desktop_shutdown.rs  desktop/src-tauri/src/desktop_shutdown/
desktop/src-tauri/src/future_login.rs   desktop/src-tauri/src/no_gui.rs
desktop/src-tauri/src/bin/              docs/testing/module-tauri-shell.md
```

Peer units own the rest of the crate: `docs/testing/module-tauri.md`
(bridge/core/remote/terminal) is a *different* record and is not edited here.
This file is the shell's record: the two process entry points, the Tauri
builder/run shell, the macOS menu and power hooks, host hydration, the instance
lock, desktop shutdown coordination and the FutureOS login poll.

## 1. Run identity and the exact measurement

Revision `8449acf6` (the campaign baseline), macOS arm64 (Darwin 25.x), Rust
1.97.0 (`rust-toolchain.toml`). The crate is **not a workspace member** and is
built in **two feature flavors**; a single-flavor number is not a coverage
number for this crate, so both runs are made and **one** merged report is
rendered from their profraw:

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-shell
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder

cargo llvm-cov clean --manifest-path Cargo.toml
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml \
    --no-default-features --features headless -- --test-threads=1
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tauri-shell-lcov-final.info
```

`--test-threads=1` is mandatory on this host (the lib suite deadlocks in
parallel); it does **not** put the test body on the main thread — §4.2 rests on
that fact and on the probe in §6.2. The measured report for this unit is
`/tmp/tauri-shell-lcov-final.info` (`cargo llvm-cov report` exit 0), rendered from the
profraw of both runs above.

Per-line work list (the tool never reads the `summary` block):

```bash
python3 /Users/geilige/future-os/.worktrees/cov100-r2/.future/cov100/covlines.py \
    detail /tmp/tauri-shell-lcov-final.info --under desktop/src-tauri/src/lib.rs
# …one --under per file in the scope above
```

## 2. Before and after, per file

**Revision note:** this section's "after" column is the **first** segment's measurement
(`/tmp/tauri-shell-lcov-final.info`, revision `8449acf6` + the first tests, 318 lines).
The current numbers are in §S4.2; segment 3's are in §S3.2. Read this table as the
historical first step, not as the state of the tree.

Uncovered lines (`DA:0`) in this unit's scope: the `8449acf6` baseline
(`.future/cov100/baseline/tauri-merged-lcov.info`) versus the **after**
measurement of this segment (`/tmp/tauri-shell-lcov-final.info`, run identity in
§1):

| file | before | after |
|---|---|---|
| `src/lib.rs` | 203 | 181 |
| `src/menu.rs` | 48 | 48 |
| `src/launch.rs` | 38 | 15 |
| `src/main.rs` | 28 | 21 |
| `src/desktop_shutdown.rs` | 20 | 18 |
| `src/future_login.rs` | 18 | 3 |
| `src/desktop_shutdown/coordinator.rs` | 15 | 9 |
| `src/macos_power.rs` | 10 | 10 |
| `src/runtime.rs` | 8 | 4 |
| `src/instance.rs` | 7 | 7 |
| `src/host_environment.rs` | 0 | 1 |
| `src/bin/futureos-headless.rs` | 5 | 1 |
| **total** | **400** | **318** |

Measured (non-zero) lines in the same scope: **3015 → 3479**. Of the 400 baseline
`DA:0` lines, **82 are now covered**; the residual 318 = 1 line this unit's own
test introduced (§6.6) + 1 line that moved for a reason outside this unit (§6.7)
+ 316 others, all classified in §4.

`desktop_shutdown.rs` −2 and `coordinator.rs` −6 are the two shutdown files where
the new fixtures landed (`reconcile_owned_runs`'s `continue`, `stop_with_runtime`'s
closure, the runtime-builder closure, the quit barrier). `instance.rs` is
unchanged at 7 in total but *different lines*: the FIFO fixture covered the
non-contended `flock` error (61–63) and the new test's own multi-line `assert_eq!`
arguments added two (§6.6).

`host_environment.rs` **0 → 1** is a move in the wrong direction and is not
hidden (§6.7). `linux_power.rs`, `windows_power.rs` and `no_gui.rs` are absent
or empty here by platform/feature (see `module-tauri.md` §5.1 and
`docs/testing/platform-coverage.md`).

Suite in the after measurement (`gui` flavor lib, `--no-fail-fast`):
**1950 passed / 3 failed / 12 ignored**; headless flavor lib **1584 passed /
5 failed**. Every failure is a pre-existing `remote_peer::transfer_tests::*` /
`remote_peer::runtime_tests::*` host failure (port 8022 web server, "the test web
port never freed") — the same set the baseline documented, none in this unit's
scope. Whole-crate run of both bin targets and `tests/headless_cli.rs` is green.

## 3. Tests added by this unit

All assertions are behaviour assertions: each one fails if the line it covers
changes meaning. `#[cfg(not(target_os = "linux"))]` appears only where the flag
under test is Linux-only *by design* (the helper would *run* on Linux instead of
being refused) — the Linux arms are §4 rows, not skipped tests.

| file | test | behaviour pinned |
|---|---|---|
| `lib.rs` | `helper_dispatch_tests::an_agent_subcommand_prefix_reaches_the_same_helper_request` | the `agent` subcommand token is stripped, so `agent --linux-sandbox-helper …` and `--linux-sandbox-helper …` dispatch identically |
| `lib.rs` | `gui::macos_dock_icon_tests::dock_icon_setup_is_a_noop_without_the_main_thread` | off the main thread `MainThreadMarker::new()` is `None` and `set_macos_dock_icon` returns without an AppKit call |
| `lib.rs` | `gui::runtime_update_tests::emit_via_helpers_route_both_arms` (extended) | `emit_remote_re_pair` routes through the optional handle: `Some` emits, `None` (the process-global handle in tests) is a no-op |
| `lib.rs` | `gui::runtime_update_tests::runtime_update_drain_loop_covers_every_receive_exit` | all four exits of the drain loop: idle `Timeout` → continue, a burst that outlives the 16 ms window (`remaining.is_zero()` → break), a window that closes empty, `Disconnected` → return |
| `main.rs` | `configure_environment_suppresses_macos_activity_logs` | `OS_ACTIVITY_MODE=disable` is really set (the previous test only took the function's address) |
| `main.rs` | `a_sandbox_helper_request_is_dispatched_before_any_startup` | the real launcher exits 1 with the helper's refusal on stderr and creates no data directory — spawn, because libtest owns `main` |
| `bin/futureos-headless.rs` | `a_sandbox_helper_request_is_dispatched_before_any_startup` | same contract for the standalone entry point |
| `launch.rs` | `prepare_host_environment_is_idempotent` | the second hydration answers exactly what the first one did |
| `launch.rs` | `the_reply_is_either_the_invitation_or_why_there_is_none` | the reply applies the headless invitation rule: a v2 link becomes the invitation, anything else becomes the phase+reason message |
| `launch.rs` | `the_notice_never_reports_success_without_an_invitation` (extended) | a link with no expiry still prints a complete notice ("It expires shortly") |
| `launch.rs` | `the_wrapper_reads_a_real_answer_and_gives_up_on_a_dead_hand_off` | the production wrapper reads a real reply file (answer already there → `Ok`) and reports a hand-off process that died without answering instead of waiting out 60 s |
| `lib.rs` | `gui::startup_paths_tests::a_re_pair_answer_is_delivered_to_the_waiting_file` | the re-pair outcome really reaches the file another process waits on, and a *failed* re-pair is delivered as a reason (no link), not as silence |
| `lib.rs` | `gui::startup_paths_tests::an_undeliverable_re_pair_answer_leaves_the_target_alone` | a delivery that cannot be written is reported and leaves the named target untouched (no partial file, no replacement) |
| `lib.rs` | `gui::startup_paths_tests::print_re_pair_notice_writes_the_launcher_notice` | the terminal delivery prints the launcher's notice verbatim (both audiences read the same text) and propagates a writer failure |
| `runtime.rs` | `a_native_job_that_outlives_the_budget_fails_the_cleanup_barrier` | a non-abortable native job fails the barrier with "did not stop before cleanup deadline" |
| `runtime.rs` | `a_panicking_native_job_is_reported_at_the_barrier` | a panicking native job surfaces as "Desktop native background job panicked" |
| `future_login.rs` | `broken_agent_endpoint_restores_a_previous_value` | the endpoint override is restored to its previous value, not just removed |
| `future_login.rs` | `auth_state_is_unavailable_when_the_credential_cannot_be_read` | an unreadable credential file is `Unavailable` (recoverable), not `SignedOut` |
| `future_login.rs` | `poll_undecodable_error_body_names_the_http_status` | a non-retryable error status with a non-JSON body reports `HTTP 400` + the decode error |
| `desktop_shutdown.rs` | `unconfirmed_runs_report_the_transport_outcome` | a tonic status and a per-command timeout both become the run's recorded reason (the "no verified terminal state" default is replaced) |
| `desktop_shutdown/coordinator.rs` | `a_quit_cannot_start_a_second_stop` | (a) a quit is refused while another maintenance action owns the barrier, (b) a repeat quit is answered with the *same* operation |
| `desktop_shutdown/coordinator.rs` | `a_stop_with_a_budget_reaches_the_runtime_builder_and_finishes` | a stop with a live budget really builds its cleanup runtime (exactly once, counted) and publishes a terminal state — the property the expired-budget fixture cannot show |
| `desktop_shutdown/coordinator.rs` | `an_expired_budget_can_never_publish_cleanup_success` (fixture fixed) | the test now starts from a reset registry and passes a *named* runtime builder, so its own worker really runs — it previously joined a sibling test's operation and proved nothing |
| `lib.rs` | `gui::startup_paths_tests::a_failed_re_pair_is_delivered_as_a_reason` | a re-pair whose *revocation* fails (the credential file's directory is read-only) is delivered as that reason — asserted by text — so the waiting launcher is answered instead of timing out |
| `lib.rs` | `gui::startup_paths_tests::print_re_pair_notice_writes_the_launcher_notice` (extended) | both writer failures are reported: a closed sink *and* a sink that accepts the notice then refuses to flush it |
| `desktop_shutdown.rs` | `reconcile_keeps_runs_whose_state_could_not_be_read` | a transport failure while reading a run's state leaves the run owned (an unread state is not a terminal state) |
| `instance.rs` | `a_lock_failure_that_is_not_contention_is_not_reported_as_another_instance` | a `flock` failure that is not contention (ENOTSUP on a FIFO) names the directory and the OS error and never claims "already running" |
| `lib.rs` | `gui::startup_paths_tests::a_failed_re_pair_is_delivered_as_a_reason` | a re-pair whose *revocation* fails is delivered as that reason (asserted by text), not as silence — the waiting launcher is answered either way |

No assertion was weakened and no production guard or `#[allow]` was added; the
only production changes are the two seams in §5.1 (both behaviour-preserving
extractions of an existing body).

## 4. Residual uncovered lines (318)

**Revision note (kept for the historical record):** the 318 rows below belong to the
first segment's revision. This section is **not** the current ledger — §S4.4 is — and its
line numbers do not describe the working tree. It is left in place because Segment 2's
`§4.11` row table and the campaign's review unit cite these rows by number; deleting them
would make those citations unfalsifiable. Every later segment says which rows it closed.

Categories are the four in `docs/testing/waiver-ledger.md`. Line numbers are the
**post-change** ones (`/tmp/tauri-shell-lcov-final.info`); the groups are disjoint
and sum to exactly 318. Where a line is untestable only because of the code's
current shape, §5 names the smallest seam; §4.11 lists what is *not* waived —
reachable lines whose fixture I identified but did not land before the
measurement window closed, kept out of the waived set on purpose.

| group | lines | category |
|---|---|---|
| 4.1 the real Tauri/Wry launch | 158 | `unreachable-in-this-environment` |
| 4.2 macOS main-thread-only | 53 | `unreachable-in-this-environment` |
| 4.3 macOS power notifications | 10 | `unreachable-in-this-environment` |
| 4.4 the launcher's GUI/re-pair process bodies | 35 | `unreachable-in-this-environment` |
| 4.5 the Linux-only helper success arm | 2 | `platform-unmeasured` |
| 4.6 `cfg(not(test))`-excluded shutdown blocks | 11 | `unreachable-by-construction` |
| 4.7 the `flock` re-acquire window | 5 | `unreachable-in-this-environment` |
| 4.8 client-stack and defensive arms | 3 | `unreachable-in-this-environment` |
| 4.9 `non_exhaustive`, never-evaluated and fixture-only arms | 9 | `unreachable-by-construction` |
| 4.10 region attributed to a brace | 3 | `attribution-artifact` |
| **waived total** | **289** | |
| 4.11 identified rework (not waived) | 29 | — |

### 4.1 The real Tauri/Wry launch — `unreachable-in-this-environment` (158)

`lib.rs` 209–215, 811, 842, 888–890, 894, 1185–1191, 1197, 1199–1241,
1245–1247, 1249–1332; `desktop_shutdown/coordinator.rs` 50.

Mechanism: these lines are only reachable while a **real** Tauri application is
running, and that cannot be created in a libtest process on macOS.
`Builder::<Wry>::build` creates the Wry event loop and — because
`enable_macos_default_menu` is on for the real app — the default **muda** menu
(`tauri-2.11.5/src/app.rs:2243-2249`), both main-thread-only; libtest runs every
test body on a spawned worker thread (§6.2), and `mock_app()`/`mock_builder()`
only avoid the *runtime*, they do not move the test body. Lines 894–900 also need
`tauri::generate_context!()` (the packaged frontend assets) at runtime, and
`app.run()` never returns, so no in-process test can observe the run loop or its
`RunEvent` arms (1199–1241). §5.2 sketches the extraction that would bring ~75 of
these (the builder chain and `start_backend_background`) into reach, and §5.6 the
one pure function for the coordinator's emitted snapshot. `coordinator.rs:50` is
the `APP_HANDLE.get()` arm of the progress emitter: the process-global is a
`OnceLock<AppHandle<Wry>>` filled only by that same real launch (the mock-handle
arms of its siblings *are* covered, §3).

### 4.2 macOS main-thread-only — `unreachable-in-this-environment` (53)

`menu.rs` 26–78 (48) and `lib.rs` 474–478 (5).

Mechanism: `muda`'s item/submenu constructors assert that they run on the macOS
main thread, and `NSApplication::sharedApplication` /
`setApplicationIconImage` are main-thread-only AppKit calls. libtest runs every
test body on a spawned worker thread — measured, not assumed: a two-line probe
crate prints `pthread_main_np() == 0` inside a `#[test]` even with
`--test-threads=1` (§6.2), and the dock-icon test asserts that precondition before
calling the guard. The previous campaign made the same call for `menu.rs` after
trying a generic-`Runtime` refactor with `mock_app()`; it is not repeated here
(§5.3 says what a working harness would need). The *guard* half of the Dock-icon
path is covered by this unit (§3).

### 4.3 macOS power notifications — `unreachable-in-this-environment` (10)

`macos_power.rs` 21–32: the three `RcBlock` bodies run on
`NSWorkspaceWillSleep` / `NSWorkspaceDidWake` / `NSWorkspaceWillPowerOff`, i.e.
only when the machine really sleeps, wakes or powers off. The registration path
itself is covered by the existing test. §5.4 has the seam and the real blocker.

### 4.4 The launcher's GUI and re-pair process bodies — `unreachable-in-this-environment` (35)

`main.rs` 29, 33–34, 38–48, 51, 58, 70–75 (20) and `launch.rs` 330–347 (15).

Mechanism: these are the bodies of a *process*, not of a library function.
`main.rs` 29–51 needs `stdin`/`stdout` to be a terminal *and* another Desktop to
own the data directory *and* that Desktop to answer the hand-off; 58 and 70–75
launch the real GUI (`start_desktop` → `run_with`, §4.1). `launch.rs`
`forward_re_pair` spawns `std::env::current_exe()` — inside the test harness that
is the *test binary*, which would re-enter libtest instead of a launcher. §5.7
gives the fixture (a real pty plus a held lock) that would cover 29–38 and leaves
the answered `Ok` arm (39–48) in this group either way.

### 4.5 The Linux-only helper success arm — `platform-unmeasured` (2)

`main.rs` 9 and `bin/futureos-headless.rs` 11 — the `Ok(()) => SUCCESS` arm of the
sandbox-helper dispatch. `dispatch_agent_helper` only returns `Ok` when the helper
really runs, and `agent/src/cli.rs:213-218` refuses the request with `bail!` on
every non-Linux target. The authoritative OS is **Linux**; there the same spawn
fixtures would additionally need a *valid* helper request, which this host cannot
produce. The refusal arms themselves are covered (§3).

### 4.6 `cfg(not(test))`-excluded shutdown blocks — `unreachable-by-construction` (11)

`desktop_shutdown.rs` 221–223 and 267–274, both
`#[cfg(all(feature = "gui", not(test)))]`: no test build contains them, so no test
can execute them. They appear in the report only through the crate's second,
non-`cfg(test)` compilation of the lib (the copy the bin targets link), where they
are reachable solely from a real Desktop shutdown that owns a terminal manager.
Invariant: *every* measurement build excludes the lines by `cfg`; removing the
test exclusion would put the terminal-manager freeze/shutdown into every test that
stops the Desktop.

### 4.7 The `flock` re-acquire window — `unreachable-in-this-environment` (5)

`instance.rs` 164–168: the transient-failure arm of a 2-second re-acquire loop,
reachable only in the fork/exec window the test's own comment describes (a child
inherits the open file description before it execs). It cannot be ordered from the
test; the tests that run the loop take the `Ok` arm on the first attempt.

### 4.8 Client-stack and defensive arms — `unreachable-in-this-environment` (3)

`future_login.rs` 354–355 and 482. `is_retryable_poll_error` (connect / timeout /
request / body / redirect) is evaluated on `response.json()`'s error; on this host
that error is always reqwest's `Kind::Decode` — three fixtures were tried (short
`Content-Length`, malformed chunked framing, `SO_LINGER=0` RST) and all three land
in the `malformed` arm (§6.5). The connect kind is handled earlier by `send()`'s
own arm, which is covered; no client timeout is configured and the URL is
validated before the request, so no other kind can arise on this path. 482 is
`log_poll_transport_error`'s "no causes" formatting branch: every error that
reaches it is constructed by reqwest around a source (hyper/io/serde), so the
`causes.is_empty()` case is defensive.

### 4.9 `non_exhaustive`, never-evaluated and fixture-only arms — `unreachable-by-construction` (9)

* `lib.rs:1244` — the `_ => {}` arm of the `RunEvent` match. `RunEvent` is
  `#[non_exhaustive]`, the arm cannot be deleted, and no variant reaches it.
* `lib.rs:1346`, `desktop_shutdown.rs:263` and `:460` — see §4.10 (brace
  attribution).
* `lib.rs:1670` — the burst producer's early `return` (send failed): it can only
  run if the drain loop already returned, which this test's design forbids before
  the burst ends.
* `lib.rs:1777-1779` — `Failed::flush`'s `Ok(())`: the two writer cases are
  "write fails" (never reaches flush) and "flush fails" (`Ok` not taken), so this
  line is unreachable from the fixture as written (§6.6 flags the fix).
* `runtime.rs:248-252` — the two closure bodies in
  `frozen_registry_never_polls_late_work_or_starts_native_jobs`: the test asserts
  the work never runs, so executing them would fail it.

### 4.10 Region attributed to a brace — `attribution-artifact` (3)

`desktop_shutdown.rs` 263 and 460, `lib.rs` 1346. Each is the closing brace of a
block whose body *is* covered: 263 closes the `if !owns_agent()` block (260–262
covered), 460 closes the `if request.r#type != ..` block (457–459 covered), 1346
closes `if !enabled { return; }` (1344–1345 covered). llvm-cov gives each block-end
region a zero counter; all three were `DA:0` in the baseline too, and 1346 sits in
a file this unit did not otherwise change its numbering in.

### 4.11 Identified rework — not waived (29)

Reachable lines with an identified fixture that I did not land before the
measurement window closed. A successor should do these first; they are the
difference between this unit's 318 and the ~289 that are structurally waived.

| lines | why it is not covered | fixture / fix |
|---|---|---|
| `instance.rs` 196–197 | **self-inflicted**: the new FIFO test's multi-line `assert_eq!` arguments are evaluated only on failure (taxonomy (e)) | hoist `lock_path.display()` / `last_os_error()` into `let` bindings and keep the `assert!` on one line |
| `desktop_shutdown.rs` 589 | same shape, in `unconfirmed_runs_report_the_transport_outcome` (`errors[0]` argument) | hoist the binding |
| `lib.rs` 833 | the inner coalescing loop's `remaining.is_zero()` break needs a burst that spans the whole 16 ms window; the steady-stream fixture did not reliably reach it | send the burst from two threads, or lengthen the window by sending until a rendezvous flag |
| `lib.rs` 1384–1389 | `RePairDelivery::Terminal` (the launcher-less stdout delivery) is only spawned from `start_backend_background` | one test calling `spawn_remote_re_pair(RePairDelivery::Terminal)` — the notice path itself is already asserted via `print_re_pair_notice` |
| `lib.rs` 1348–1359 | `spawn_remote_auto_connect`'s spawned closure: the only existing fixture returns at the `enabled` gate | persist pairing creds + `auto_connect_remote` with a broken agent endpoint, then assert the deferred/failed report |
| `desktop_shutdown.rs` 256–258 | the remote-shutdown deadline arm: with an expired budget the zero-timeout still lets an instantly-ready `stop_gracefully` win, so the arm needs a *slow* stop | drive it with the remote tests' `FakeNats` fixture, or make the deadline deterministic through §5.6's seam |
| `desktop_shutdown/coordinator.rs` 61, 165–166, 183–187 | the deadline rewrite needs the worker to finish *after* the deadline while the watchdog has not yet published; the watchdog/worker *spawn* failures need `thread::Builder::spawn` to fail (EMFILE/EAGAIN) | seam §5.6 (pure deadline rewrite) or an injected spawner — production-shape changes, not fixtures |
| `host_environment.rs` 28 | `start_logging`'s `report(&error)` arm runs only when `cli_tool::resolve()` fails; **it moved from covered to uncovered between the two measurements** (§6.7) | one test calling `host_environment::start_logging()` with a PATH where the CLI cannot resolve |

Rows 6–7 are the ones I would argue for waiving under
`unreachable-in-this-environment` if no seam is wanted (they need an ordering
fixture and an OS-level thread refusal). They are deliberately left un-waived so
a reviewer sees them rather than having to falsify a category.

## 5. Seams proposed

A seam is proposed only where the *current shape* of the code is the only reason a
line cannot be asserted. Nothing below was invented silently: 5.1 is implemented
(tiny and behaviour-preserving), 5.2–5.7 are proposals for whoever owns the code
next, with the lines each one would unlock.

### 5.1 Implemented (this segment): the terminal-delivery notice and the emit route

Two behaviour-preserving extractions, both in `gui`:
`gui::print_re_pair_notice<W: Write>` (below) and
`gui::emit_remote_re_pair_via(Option<&AppHandle<R>>)`, which mirrors the file's
existing `*_via` helpers so `emit_remote_re_pair`'s two arms are testable with a
mock handle.

#### `gui::print_re_pair_notice<W: Write>` (3 lines + 2 arm lines)

`RePairDelivery::Terminal` wrote `invitation_notice(reply)` directly to
`std::io::stdout()`, so the notice path could be executed but never asserted —
libtest captures the process's stdout. The writer is now a parameter
(`print_re_pair_notice(&mut W, &Reply)`), the delivery arm passes
`std::io::stdout()`, and the notice text, the flush and the error propagation are
asserted against a `Vec<u8>` and a closed writer. Behaviour is unchanged: the
arm still ignores a failed write (`let _ = …`), and the same notice text reaches
the same sink.

### 5.2 The GUI shell: make the startup path generic over the runtime

Lines that need it: the `run_with` builder chain (§4.1) and
`start_backend_background` (§4.3). Both are `Wry`-typed and only reachable from
`app.run()`. Two mechanical extractions would unlock them:

1. `fn gui_builder<R: Runtime>(launch: LaunchOptions) -> tauri::Builder<R>` — the
   chain from `.plugin(tauri_plugin_single_instance::init(..))` to
   `.invoke_handler(tauri::generate_handler![..])`. Every plugin used here is
   already generic over `R`; a test can pass `tauri::test::mock_builder()` and
   assert the *registration* (the closure bodies still need real events).
   Unlocks the chain's own ~40 lines.
2. `fn start_backend_background<R: Runtime>(app: tauri::AppHandle<R>, launch)` —
   it uses the handle only to call `scheduler::start` and `tasks::start`, which
   are already `R: Runtime`. A test can pass `mock_app().handle()`.
   Unlocks ~35 lines.

What it does **not** fix: the `setup`, `on_window_event`, `on_menu_event` and
run-event closure bodies (they need real window/menu events), and any line that
needs `Builder::<Wry>::build` — on macOS that creates the Wry event loop and the
**default muda menu**, both main-thread-only (§4.1, §6.2).

Risk note for whoever implements 5.2(2): the function also spawns
`scheduler::start`/`tasks::start` tick loops and a **non-abortable**
`spawn_native(shadow_review::run_startup_maintenance)`; a test must let those
finish (or the `HomeGuard` teardown panics) before it can be trusted in the
suite. That is why it is proposed, not landed, in this segment: the measurement
window was already open.

### 5.3 The macOS menu: nothing smaller than a main-thread harness

`menu::build_macos_menu` is not untestable because of a missing injection point —
it is untestable because muda asserts `NSThread::isMainThread` during
construction. The only shapes that would work are a helper *process* whose real
main thread constructs the menu (the crate already spawns helpers this way:
`remote_peer::multi_host_tests::host_process_child`), or a `tauri::test` harness
that runs the test body on the main thread. Neither is a seam in this file.

### 5.4 `macos_power`: name the three notifications

`fn on_system_sleep()` / `on_system_resume()` / `on_system_power_off()` extracted
from the three `RcBlock` bodies would make their *bodies* callable. The blocker is
not the seam: those bodies call `remote::handle_system_suspend` /
`handle_system_resume` / `notify_mobile_disconnect`, which mutate the
process-global `remote::SUPERVISOR` (suspend cancels the generation and latches
`last_error_code`). The remote module drives them from its own fixture
(`remote/tests.rs`, `FakeNats` + `install_state`); a shell test would have to
replicate that fixture or corrupt every later remote test in the process. Until
that fixture is shared, this file stays waived — the same call the previous
campaign recorded for `windows_power.rs`.

### 5.5 `instance.rs`: inject the lock operation

`InstanceGuard::at` maps two different `flock` failures through one `map_err`; the
non-contended branch needs a `File` whose lock fails for another reason. Two
routes, both verified against this host's behaviour:

* smallest seam: `fn at_with(directory, lock: impl FnOnce(&File) -> Result<(), std::io::Error>)`
  (or a two-method `Lock` trait) so a test injects `ENOTSUP` and asserts the
  *other* message;
* fixture-only (no production change): make `desktop.lock` a **FIFO** —
  `libc::mkfifo` + `flock` probed on this host returns `ENOTSUP` (errno 45) while
  `open(fifo, O_RDWR)` succeeds, which is exactly the non-contended failure the
  branch maps. It is platform-gated (`flock` on a FIFO is not `ENOTSUP` on every
  OS, so it would need `#[cfg(target_os = "macos")]`) and was **not adopted in
  this segment** because the measurement window was already open when the probe
  finished; a successor can land it in minutes.

### 5.6 `coordinator::Operation::finish`: extract the deadline rewrite

`if result.is_ok() && Instant::now() >= self.deadline { Err(format!(…)) }` is
reachable only when the worker succeeds *after* the deadline while the watchdog
thread has not yet published its own failure — a scheduling race, not an input.
A pure `fn enforce_deadline(result, now, deadline, stage) -> Result<(), String>`
would make the rewrite (and its message) a unit test.

### 5.7 `main.rs` re-pair launcher: a TTY fixture, or an injected launcher

Lines 29–48 need `require_terminal()` to succeed *and* a lock owner to hand off
to. The smallest fixture is a real pty (`portable-pty` is already a dependency)
plus a lock held by the test; the `Ok` arm (a Desktop that answers the hand-off)
needs a running graphical Desktop and stays in §4.2 either way.

## 6. Findings

### 6.1 A test that proved nothing (fixed here)

`desktop_shutdown::coordinator::tests::an_expired_budget_can_never_publish_cleanup_success`
passed a `Duration::ZERO` budget and asserted a deadline failure — but
`begin_with_budget` **returns the operation already in the registry**, and a
sibling test had left one behind. The test therefore joined that sibling's
operation: its own runtime-builder closure never ran (visible as four `DA:0`
lines inside the test), so its assertion was about someone else's cleanup. The
fixture now starts from `reset_for_test()` and ends with it, and the same
pattern was applied to the new quit test. This is a test-side bug, not a
production one; nothing was weakened to fix it.

### 6.2 libtest never runs a test body on the main thread

Measured with a two-line probe crate: `pthread_main_np()` returns **0** inside a
`#[test]` even with `--test-threads=1` (libtest spawns a thread per test either
way). Everything that macOS requires on the main thread — `muda` menu
construction, `NSApplication` — is therefore out of reach in-process, and that is
the mechanism behind §4.1/§4.4/§4.5. The dependency `tauri::test::mock_app()` does
not help: it only avoids creating a real runtime, it does not move the test body.

### 6.3 `CARGO_BIN_EXE_*` is not defined for a bin's own test target

Cargo defines it for integration tests and benches only, so the two new spawn
tests inside `src/main.rs` and `src/bin/futureos-headless.rs` locate the launcher
the way cargo lays it out: `<target>/<profile>/deps/<harness>` → the launcher one
directory up. Consequence worth knowing: those tests need the package's normal
test build (`cargo test` builds every bin because `tests/headless_cli.rs`
exists); `cargo test --bin futureos` alone builds nothing to spawn and the test
fails loudly instead of silently passing.

### 6.4 The bin entry points were already covered by spawning — only the arms nobody passed were left

The baseline shows `main.rs:7` executed **13 times** (from
`tests/headless_cli.rs`): `--help`, four invalid argument shapes, `--re-pair`
without a terminal, the headless flavor's no-GUI message. The residual 28 lines
were exactly the arms no fixture reached: the helper dispatch (`--linux-sandbox-helper`)
and the whole re-pair hand-off. That is why this unit's main.rs work is two spawn
cases rather than a refactor.

### 6.5 This host's client stack reports body-transfer failures as decode errors

`future_login::poll`'s retryable-error arm is `Err(error) if
is_retryable_poll_error(&error)` on the `response.json()` result
(connect/timeout/request/body/redirect). Three fixtures were tried against a raw
`TcpListener` — `Content-Length: 4096` with a short body, a chunked stream with a
non-hexadecimal chunk size, and `SO_LINGER=0` (RST) mid-body — and **all three**
end in the `malformed` arm with `error decoding response body` (reqwest
`Kind::Decode`), i.e. `is_body()`/`is_connect()` never see them. The retry that
these kinds drive is still exercised through `send()`'s own error arm
(`poll_network_error`), which is covered. Recorded because the obvious fixture
*looks* like it should hit this line and does not.

### 6.6 Two `DA:0` lines this unit's own tests introduced (taxonomy (e))

`instance.rs:196-197` and `desktop_shutdown.rs:589` are the multi-line
`assert_eq!`/`assert!` *arguments* of tests added here: the format arguments are
evaluated only when the assertion fails, so a passing run leaves them `DA:0`
(the repo's documented taxonomy (e), `.future/memory/coverage-llvm-cov.md`). The
fix is one line each — hoist the argument into a `let` binding and keep the
assertion on one line, the shape `main.rs`/`bin/futureos-headless.rs` already use.
It is **not** applied here on purpose: editing the sources after taking the
measurement would shift every line number in §4/§8 and the record would no longer
describe a revision anyone measured. A successor should apply the hoist and
re-measure (it is worth exactly 2 lines, so it rides along with the §4.11 work).

### 6.7 One line moved from covered to uncovered, for a reason outside this unit

`host_environment.rs:28` (`start_logging`'s `report(&error)` arm, taken when
`cli_tool::resolve()` fails) was **covered** in the `8449acf6` baseline and is
**`DA:0`** in the after measurement. Two things changed under it and neither is in
this unit's scope: `cli_tool.rs` was rewritten by a peer unit during this campaign
(it grows by ~186 lines in this worktree), and whether `resolve()` fails during
the suite depends on the process `PATH` at that moment. Reported rather than
hidden, because a ±1 line that nobody mentions is how a before/after number stops
being trustworthy. The fixture that would pin it from this unit's side is in
§4.11 (call `start_logging()` with a PATH where the CLI cannot resolve); it is
reachable and should be written, not waived.

## 7. Reproduction

```bash
# 1. measurement (both flavors, one merged report) — §1, ~45 min on this host:
#    recreate the four cargo llvm-cov commands verbatim from §1 (clean → gui →
#    headless → report); the after numbers in §2 and every line in §8 come from
#    that run's /tmp/tauri-shell-lcov-final.info

# 2. the work list for any file in this unit's scope
python3 .future/cov100/covlines.py detail /tmp/tauri-shell-lcov-final.info \
    --under desktop/src-tauri/src/lib.rs

# 3. this unit's tests, fast (uninstrumented, no llvm-cov)
cd desktop/src-tauri && cargo test --lib -- --test-threads=1 agent_subcommand_prefix
cd desktop/src-tauri && cargo test --bin futureos -- --test-threads=1 tests::
cd desktop/src-tauri && cargo test --no-default-features --features headless \
    --bin futureos-headless -- --test-threads=1 tests::

# 4. the whole crate (the acceptance run; `remote_peer::transfer_tests::*` are the
#    documented pre-existing host failures)
cargo test --manifest-path desktop/src-tauri/Cargo.toml --no-fail-fast -- --test-threads=1
```

## 8. Per-line index of the residual (318 rows)

**Revision note:** this index describes the first segment's revision (318 rows). The
current per-line index is §S4.5 (242 rows) and segment 3's is §S3.4's. Kept because
Segment 2/3 cite rows here by number.

One row per residual line, generated from `/tmp/tauri-shell-lcov-final.info` and
the groups in §4, so a reviewer can check any single line without re-deriving the
classification:

| line | group | category |
|---|---|---|
| `src/bin/futureos-headless.rs:11` | 4.5 | `platform-unmeasured` |
| `src/desktop_shutdown.rs:221` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:222` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:223` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:256` | 4.11 | `REWORK` |
| `src/desktop_shutdown.rs:257` | 4.11 | `REWORK` |
| `src/desktop_shutdown.rs:258` | 4.11 | `REWORK` |
| `src/desktop_shutdown.rs:263` | 4.10 | `attribution-artifact` |
| `src/desktop_shutdown.rs:267` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:268` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:269` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:270` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:271` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:272` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:273` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:274` | 4.6 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:460` | 4.10 | `attribution-artifact` |
| `src/desktop_shutdown.rs:501` | 4.9 | `unreachable-by-construction` |
| `src/desktop_shutdown.rs:589` | 4.11 | `SELF-INFLICTED test lines` |
| `src/desktop_shutdown/coordinator.rs:50` | 4.1 | `unreachable-in-this-environment` |
| `src/desktop_shutdown/coordinator.rs:61` | 4.11 | `REWORK` |
| `src/desktop_shutdown/coordinator.rs:165` | 4.11 | `REWORK` |
| `src/desktop_shutdown/coordinator.rs:166` | 4.11 | `REWORK` |
| `src/desktop_shutdown/coordinator.rs:183` | 4.11 | `REWORK` |
| `src/desktop_shutdown/coordinator.rs:184` | 4.11 | `REWORK` |
| `src/desktop_shutdown/coordinator.rs:185` | 4.11 | `REWORK` |
| `src/desktop_shutdown/coordinator.rs:186` | 4.11 | `REWORK` |
| `src/desktop_shutdown/coordinator.rs:187` | 4.11 | `REWORK` |
| `src/future_login.rs:354` | 4.8 | `unreachable-in-this-environment` |
| `src/future_login.rs:355` | 4.8 | `unreachable-in-this-environment` |
| `src/future_login.rs:482` | 4.8 | `unreachable-in-this-environment` |
| `src/host_environment.rs:28` | 4.11 | `PEER-INTERACTION (cli_tool scope)` |
| `src/instance.rs:164` | 4.7 | `unreachable-in-this-environment` |
| `src/instance.rs:165` | 4.7 | `unreachable-in-this-environment` |
| `src/instance.rs:166` | 4.7 | `unreachable-in-this-environment` |
| `src/instance.rs:167` | 4.7 | `unreachable-in-this-environment` |
| `src/instance.rs:168` | 4.7 | `unreachable-in-this-environment` |
| `src/instance.rs:196` | 4.11 | `SELF-INFLICTED test lines` |
| `src/instance.rs:197` | 4.11 | `SELF-INFLICTED test lines` |
| `src/launch.rs:330` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:331` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:332` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:333` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:334` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:338` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:339` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:340` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:341` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:342` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:343` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:344` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:345` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:346` | 4.4 | `unreachable-in-this-environment` |
| `src/launch.rs:347` | 4.4 | `unreachable-in-this-environment` |
| `src/lib.rs:209` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:210` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:211` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:212` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:213` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:214` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:215` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:474` | 4.2 | `unreachable-in-this-environment` |
| `src/lib.rs:475` | 4.2 | `unreachable-in-this-environment` |
| `src/lib.rs:476` | 4.2 | `unreachable-in-this-environment` |
| `src/lib.rs:477` | 4.2 | `unreachable-in-this-environment` |
| `src/lib.rs:478` | 4.2 | `unreachable-in-this-environment` |
| `src/lib.rs:811` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:833` | 4.11 | `REWORK` |
| `src/lib.rs:842` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:888` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:889` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:890` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:894` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:898` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:899` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:900` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:901` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:902` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:905` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:906` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:907` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:908` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:909` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:915` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:916` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:918` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:919` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:920` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:922` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:923` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:924` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:925` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:926` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:927` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:931` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:932` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:933` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:934` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:935` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:936` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:940` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:941` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:942` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:944` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:949` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:950` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:954` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:955` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:956` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:957` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:959` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:961` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:962` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:963` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:964` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:966` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:967` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:968` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:969` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:970` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:973` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:974` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:975` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:976` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:978` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:987` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:988` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:989` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:990` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:991` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:993` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:994` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1014` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1015` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1016` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1017` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1018` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1019` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1020` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1021` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1022` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1023` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1178` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1179` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1185` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1186` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1187` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1188` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1189` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1190` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1191` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1197` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1203` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1204` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1205` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1206` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1207` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1208` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1209` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1210` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1211` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1212` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1213` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1214` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1215` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1216` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1221` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1222` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1224` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1225` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1226` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1227` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1229` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1230` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1231` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1232` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1233` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1237` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1238` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1239` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1240` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1241` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1242` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1243` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1244` | 4.9 | `unreachable-by-construction` |
| `src/lib.rs:1245` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1247` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1251` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1255` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1260` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1263` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1269` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1273` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1276` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1282` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1286` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1288` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1294` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1295` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1296` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1301` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1302` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1303` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1304` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1305` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1306` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1307` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1308` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1316` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1317` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1318` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1321` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1325` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1326` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1327` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1328` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1329` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1330` | 4.1 | `unreachable-in-this-environment` |
| `src/lib.rs:1346` | 4.10 | `attribution-artifact` |
| `src/lib.rs:1348` | 4.11 | `REWORK` |
| `src/lib.rs:1349` | 4.11 | `REWORK` |
| `src/lib.rs:1350` | 4.11 | `REWORK` |
| `src/lib.rs:1351` | 4.11 | `REWORK` |
| `src/lib.rs:1353` | 4.11 | `REWORK` |
| `src/lib.rs:1357` | 4.11 | `REWORK` |
| `src/lib.rs:1359` | 4.11 | `REWORK` |
| `src/lib.rs:1384` | 4.11 | `REWORK` |
| `src/lib.rs:1385` | 4.11 | `REWORK` |
| `src/lib.rs:1386` | 4.11 | `REWORK` |
| `src/lib.rs:1387` | 4.11 | `REWORK` |
| `src/lib.rs:1388` | 4.11 | `REWORK` |
| `src/lib.rs:1389` | 4.11 | `REWORK` |
| `src/lib.rs:1670` | 4.9 | `unreachable-by-construction` |
| `src/lib.rs:1841` | 4.9 | `unreachable-by-construction` |
| `src/lib.rs:1842` | 4.9 | `unreachable-by-construction` |
| `src/macos_power.rs:21` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:22` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:23` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:24` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:26` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:27` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:29` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:30` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:31` | 4.3 | `unreachable-in-this-environment` |
| `src/macos_power.rs:32` | 4.3 | `unreachable-in-this-environment` |
| `src/main.rs:9` | 4.5 | `platform-unmeasured` |
| `src/main.rs:29` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:33` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:34` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:38` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:39` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:41` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:42` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:43` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:44` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:46` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:47` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:48` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:51` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:58` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:70` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:71` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:72` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:73` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:74` | 4.4 | `unreachable-in-this-environment` |
| `src/main.rs:75` | 4.4 | `unreachable-in-this-environment` |
| `src/menu.rs:26` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:27` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:28` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:29` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:31` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:32` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:33` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:34` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:35` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:36` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:37` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:38` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:39` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:40` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:41` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:42` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:43` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:44` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:45` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:46` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:47` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:48` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:49` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:51` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:52` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:53` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:54` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:55` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:56` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:57` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:58` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:59` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:61` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:62` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:63` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:65` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:66` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:67` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:68` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:69` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:70` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:72` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:73` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:74` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:75` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:76` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:77` | 4.2 | `unreachable-in-this-environment` |
| `src/menu.rs:78` | 4.2 | `unreachable-in-this-environment` |
| `src/runtime.rs:248` | 4.9 | `unreachable-by-construction` |
| `src/runtime.rs:249` | 4.9 | `unreachable-by-construction` |
| `src/runtime.rs:251` | 4.9 | `unreachable-by-construction` |
| `src/runtime.rs:252` | 4.9 | `unreachable-by-construction` |


# Segment 2 — the runtime seam lands, §4.11's 29 OPEN rows are closed to 12 covered / 17 registered, and the two remote-dependent rows are registered rather than faked

Status: **measured, not at target** — **270** uncovered lines in this unit's scope,
down from **318** at the end of the previous segment (and **400** at `8449acf6`).
Sections 1–8 above describe the previous revision
(`/tmp/tauri-shell-lcov-final.info`, 318 rows) and are deliberately left
untouched; this section describes the revision measured here
(`/tmp/tauri-shell-lcov-seg2b.info`), whose line numbers supersede §2/§4/§8
wherever the two disagree (the seam and the new fixtures are inserted above
`lib.rs`'s launch path, so its line numbers moved; every file's authoritative row
for *this* revision is §S2.6, and §4/§8 keep the previous revision's numbers).

What changed, in one paragraph: the two documented one-line hoists were applied
(§4.11 rows 1–2), the GUI registration and the window's background work were made
generic over the Tauri runtime — the seam the supervisor approved under PLAN §4b —
and eight fixtures (five new tests, three reworked ones) closed 12 of the 29 `OPEN`
rows; the other 17 are registered with the exact fixture that would close them.
**7 further lines (`lib.rs` 1319, 1327–1332) are deliberately left un-waived as
`REWORK`** — §S2.12; the first draft had them waived and §S2.10 measures why that was
an over-claim. So the ledger is `246 waived + 17 registered + 7 rework = 270`. No guard
was deleted, widened or `#[ignore]`d; no
assertion was weakened; nothing in `src/remote/**`, `src/commands/**`,
`src/terminal/**`, `src/remote_host/**` was touched (those are peer units' scopes —
and their files are where the remaining red in this worktree lives, §S2.7).

## S2.1 Run identity and the exact measurement

Same two-flavour instrumented run as §1, re-taken on this revision — one merged
report from both flavours' profraw, measured in place on this worktree's working
tree (the source of this segment is snapshotted as `5290059e` plus the two test
edits above it), macOS arm64 (Darwin 25.x), Rust 1.97.0, `--test-threads=1`
(mandatory on this host):

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-shell
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder   # exit 0

cargo llvm-cov clean --manifest-path Cargo.toml                                     # exit 0
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
 #   → exit 0: 2005 passed / 0 failed / 12 ignored
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml \
    --no-default-features --features headless -- --test-threads=1
 #   → exit 101: 1597 passed / 3 failed / 12 ignored (all three outside this unit — §S2.7)
cargo llvm-cov report --manifest-path Cargo.toml --lcov \
    --output-path /tmp/tauri-shell-lcov-seg2b.info                                  # exit 0
```

The whole run is the script `/tmp/tau-c2-measure.sh` (log
`/tmp/tau-c2-measure3.log`, 12:35→12:55 UTC); every step's exit code is printed
above and every one is reproduced in §S2.7. Suites in that run: `gui` flavour lib
**2005 passed / 0 failed / 12 ignored**, headless flavour lib **1597 passed / 3
failed / 12 ignored**, and every other target green (`futureos` bin 2/0,
`futureos-headless` bin 1/0, `tests/headless_cli.rs` 9/0, the spawned launcher
cases 1/0 each).

Work list, per file (the tool never reads the `summary` block):

```bash
python3 .future/cov100/covlines.py detail /tmp/tauri-shell-lcov-seg2b.info \
    --under desktop/src-tauri/src/lib.rs
python3 .future/cov100/covlines.py total  /tmp/tauri-shell-lcov-seg2b.info \
    --under desktop/src-tauri/src
```

## S2.2 Before and after, per file

Uncovered lines (`DA:0`), previous segment's merged report versus this one:

| file | previous segment (§2) | this segment |
|---|---|---|
| `src/lib.rs` | 181 | 138 |
| `src/menu.rs` | 48 | 48 |
| `src/launch.rs` | 15 | 15 |
| `src/main.rs` | 21 | 21 |
| `src/desktop_shutdown.rs` | 18 | 17 |
| `src/future_login.rs` | 3 | 3 |
| `src/desktop_shutdown/coordinator.rs` | 9 | 8 |
| `src/macos_power.rs` | 10 | 10 |
| `src/runtime.rs` | 4 | 4 |
| `src/instance.rs` | 7 | 5 |
| `src/host_environment.rs` | 1 | 0 |
| `src/bin/futureos-headless.rs` | 1 | 1 |
| **total** | **318** | **270** |

Measured (non-zero) lines in the same scope: **3479 → 3602** (`lib.rs` 803 → 886;
`launch.rs`'s 415 → 423 is this segment's `rustfmt` expansion of two pre-existing
test asserts, not new code). Per-file work list: `python3 .future/cov100/covlines.py
detail /tmp/tauri-shell-lcov-seg2b.info --under desktop/src-tauri/src/<file>`.

`host_environment.rs` **1 → 0** closes the ±1 line §6.7 could not explain: it is
**covered** by the PATH fixture §4.11 asked for. `desktop_shutdown.rs` 18 → 17 is the
hoist (§S2.5) with the two registry-flag-dependent lines staying covered (§S2.8.4).
Everything else that moved is inside `lib.rs` (the seam and the five new fixtures).

## S2.3 §4.11's 29 OPEN rows, one by one

| # | §4.11 row (previous revision's line numbers) | disposition | evidence |
|---|---|---|---|
| 1 | `instance.rs` 196–197 (2) | **covered** | hoist, §S2.5 — `a_lock_failure_that_is_not_contention_is_not_reported_as_another_instance` reaches both argument lines |
| 2 | `desktop_shutdown.rs` 589 (1) | **covered** | hoist, §S2.5 — `unconfirmed_runs_report_the_transport_outcome` now evaluates the message unconditionally |
| 3 | `lib.rs` 833 (1) | **covered** | time-bounded burst, §S2.8.1 — `runtime_update_drain_loop_covers_every_receive_exit` |
| 4 | `lib.rs` 1384–1389 (6) | **covered** | fd-1 capture, §S2.8.2 — `the_terminal_delivery_prints_the_launcher_notice_to_this_stdout` |
| 5 | `lib.rs` 1348–1359 (7) | **registered** `unreachable-in-this-environment` | §S2.8.3: the spawned auto-connect pass needs `remote::SUPERVISOR.state`, whose only installer is private to `src/remote/**` (a peer unit's scope) |
| 6 | `desktop_shutdown.rs` 256–258 (3) | **registered** `unreachable-in-this-environment` | §S2.8.3: same, plus a deliverable-presence notice that must stay pending for the whole 2 s budget |
| 7 | `coordinator.rs` 61 (1) | **covered** | `a_success_after_the_deadline_is_rewritten_into_a_failure` — the rewrite is reached by construction, §S2.4 |
| 8 | `coordinator.rs` 165–166, 183–187 (7) | **registered** `unreachable-in-this-environment` | `std::thread::Builder::spawn` returns `Err` only on an OS thread refusal (EAGAIN/ENOMEM); forcing one needs a process-wide `RLIMIT_NPROC` change that would break every sibling test in the same binary, and the names are compile-time constants with no injection point |
| 9 | `host_environment.rs` 28 (1) | **covered** | `a_host_without_a_resolvable_cli_still_starts_logging` — the PATH fixture §4.11 and §6.7 asked for |

Arithmetic, line by line, from the two merged reports: **12 row lines covered, 17
registered, 0 left unclassified**, and `318 − 48 = 270`. Of the 48 newly covered
lines, 7 are `lib.rs` rows (833 and the terminal delivery, 1384–1389), 5 are rows
1, 2, 7 and 9 in the other files, and **36 are the seam's** (§S2.4). Equivalently,
the previous segment's two sets were `289 waived + 29 open`; this one is
**`246 waived + 17 registered + 7 rework`** — i.e. the seam did not merely move the
metric, it retired 36 lines that the previous segment had recorded as unreachable,
and the 17 the seam could *not* reach are registered with the fixture that would
close them rather than left as `REWORK`. The 7 rework lines are §S2.12: the first
draft had them in S2.8's `unreachable-by-construction`, and the re-verification in
§S2.10 shows that label was an over-claim (a broken-`app.db` input reaches them), so
they are named as un-landed work instead.

## S2.4 The seam (PLAN §4b): `gui_builder<R>` and `start_backend_background<R>`

Approved as proposed in §5.2, and landed in the smallest form that compiles:

* `fn gui_builder<R: tauri::Runtime>() -> tauri::Builder<R>` holds the plugin
  registration, the menu hook and the window-event hook — the part of `run_with`'s
  chain that is runtime-generic. `run_with` calls it as
  `gui_builder::<tauri::Wry>()`.
* `fn start_backend_background<R: tauri::Runtime>(app: AppHandle<R>, launch)` — the
  window's background work (`scheduler::start`, `tasks::start`, the five
  `agent_bridge` observers, the readiness reconciliation, the maintenance job, and
  the `re_pair`/auto-connect split).

**Behaviour-identical for the real runtime:** `run_with` is the only production
caller and passes `tauri::Wry`; every item in the chain was already `R`-generic
(`tauri::Builder::<R>::new`, `Plugin<R>` initialisers, `AppHandle<R>` inside the
closures, `scheduler::start<R>`, `tasks::start<R>`); and the moved `.setup(…)` /
`.invoke_handler(…)` are stored in separate `Builder` fields, so registering them
after `gui_builder` is the same builder. No line was deleted; the two functions
gained a type parameter and a generic call site.

**Measured yield: `lib.rs` 181 → 138, i.e. −43 lines**, of which 7 are §4.11 row
lines (833 and the terminal delivery) and 36 are the registration chain plus the
window's background work. That is this record's number, not §5.2's "~40"/"~35"
estimate — those also counted closure bodies, which a mock runtime cannot execute
(no real window, menu, close request or run event). Two runs of the same revision
show those 36 as 30 covered + 6 `DA:0` and then 36 covered: the 6 (`lib.rs` 906,
910, 917, 923, 924 and one more in the same block) are adjacent one-line
statements of the plugin/menu hooks whose llvm-cov line attribution is unstable,
not lines the seam failed to reach — the two-test probe below executes the chain
itself, and those lines are `DA:0` there and covered in the full suite. The
conservative reading of the seam's yield is therefore **30 lines, ±6 of
attribution noise**, and the file-level drop is the number to hold onto.

Evidence that the seam *executes* the registration rather than merely moving it,
taken with only the two new seam tests running
(`/tmp/tau-c2-seamprobe.info`, `cargo llvm-cov --no-report … --
the_desktop_registration_builds_for_any_runtime
backend_background_work_starts_for_any_runtime`, exit 0): `lib.rs` 901
(`Builder::<R>::new`), 902 (the single-instance plugin), 925-927 (dialog, shell,
updater, the menu hook), 1345/1349-1352 (`spawn_native`, the `re_pair` branch, both
calls) all carry a non-zero counter, and `start_backend_background`'s five
observers and its 3 s readiness reconciliation run on the shared runtime.

**What it does *not* unlock, and why** — 74 of the 144 remaining `lib.rs` lines are
still Wry-only and are registered as such (§S2.6, group S2.1):

* `Builder::build` on this host creates the Wry event loop and Tauri's default
  macOS menu, and `muda` asserts the main thread — no libtest thread is the main
  thread (§6.2). So `run_with`'s own body (context, identifier, guard, logging,
  store init, `app.run(...)` and its `RunEvent` closure) cannot be reached.
* The `.setup` hook and the invoke handler stay in `run_with`: the hook publishes
  the process-global `AppHandle<Wry>` (`APP_HANDLE`), and 25 of the commands
  behind `generate_handler!` take the concrete `tauri::AppHandle` rather than
  `AppHandle<R>`, so the handler list is not runtime-generic.

Three tests, all behaviour-asserting:

| test | what it asserts |
|---|---|
| `the_desktop_registration_builds_for_any_runtime` | the same chain builds for `tauri::test::MockRuntime`, and the shell plugin's own managed state (`Shell<MockRuntime>`) really is installed by it — a chain that dropped or `Wry`-tied a plugin fails here |
| `backend_background_work_starts_for_any_runtime` | both launch shapes start the window's work on a mock handle (default → auto-connect gate; `re_pair: true` → the re-pair delivery), then drain the runtime before HOME changes |
| `a_success_after_the_deadline_is_rewritten_into_a_failure` | (coordinator, §S2.3 row 7) the deadline rewrite and the reason it publishes |

Two host facts a mock-runtime test in this crate must know, recorded because both
cost a cycle to find: `Builder::build` aborts with `muda::MenuChild can only be
created on the main thread` unless the *test* turns the default macOS menu off
(`tauri::test::mock_builder` does exactly that), and the updater plugin refuses to
initialise against a context with no `plugins.updater` section
(`PluginInitialization("updater", "invalid type: null, expected struct Config")`) —
the test inserts a minimal one.

**The MENU refactor stays rejected.** §5.3's generic-Runtime + `mock_app()` menu
harness still buys nothing: `build_macos_menu` constructs `muda` items, and
`muda::MenuChild::new` asserted `NSThread::isMainThread` in this segment's runs too
(the abort above *is* that assertion, hit at build time). I have no contrary
evidence, so §5.3 is unchanged rather than silently attempted.

## S2.5 The two hoists (§4.11 rows 1–2)

* `instance.rs`: the FIFO fixture's `assert_eq!(created, 0, "mkfifo {}: {}", lock_path.display(), std::io::Error::last_os_error())`
  is now `let mkfifo_error = format!(…)` + a one-line `assert_eq!(created, 0, "{mkfifo_error}")`.
* `desktop_shutdown.rs`: `unconfirmed_runs_report_the_transport_outcome` binds
  `let reason = errors[0].clone();` and asserts against it.

Both are the documented taxonomy-(e) shape: `assert!`'s format arguments are
evaluated only on failure, so their lines were `DA:0` in a passing run (§6.6).
Both hoists keep the same message text and the same failure behaviour. The same
trap was not re-introduced: the new fd-1 capture in `lib.rs` hoists its `dup2`
message for exactly this reason (`let dup2_failure = format!(…)`).

## S2.6 The residual (270 lines) — categories

| group | lines | category | what places the line there |
|---|---|---|---|
| S2.1 the real Tauri/Wry application — `run_with`'s own body, the run-event closure, the plugin/menu/window hook bodies | 121 | `unreachable-in-this-environment` | `Builder::<Wry>::build` creates the Wry event loop and Tauri's default macOS menu, and `muda` only allows menu construction on the main thread (§6.2: a libtest body is never the main thread). The hooks' bodies additionally need real window/menu/IPC events. `coordinator.rs:50` is the same "needs the real app handle" arm (`APP_HANDLE`). |
| S2.2 macOS main-thread-only — `menu.rs` (48) and `macos_power.rs` (10) | 58 | `unreachable-in-this-environment` | `build_macos_menu` constructs `muda` items and asserts `NSThread::isMainThread`; the power hooks are AppKit/IOKit notifications. Both were waived unchanged in §4.2/§4.3, and §S2.4 re-confirms the assertion from a second direction. |
| S2.3 the launcher *process* bodies — `launch.rs` 330–347, all of `main.rs`'s arms | 36 | `unreachable-in-this-environment` | these bodies spawn a real GUI Desktop and wait for its answer; §4.4's waiver, unchanged, no fixture exists that can become a second real Desktop on this host. |
| S2.4 the Linux-only helper success arm | 1 | `platform-unmeasured` | `bin/futureos-headless.rs:11` compiles only on Linux. |
| S2.5 `cfg(not(test))`-excluded blocks — `desktop_shutdown.rs` 221–223, 267–274 | 11 | `unreachable-by-construction` | both blocks sit behind `#[cfg(all(feature = "gui", not(test)))]`; every test build removes them, so no input reaches them. Invariant: the test cfg. |
| S2.6 the `flock` re-acquire window | 5 | `unreachable-in-this-environment` | `instance.rs` 164–168 needs a lock taken *inside* the re-acquire window; §5.4's FIFO fixture covers the other `flock` failure (61–63, now covered). Unchanged from §4.7. |
| S2.7 client-stack arms | 3 | `unreachable-in-this-environment` | `future_login.rs` 354–355, 482: §6.5's measured result — this host's stack reports body-transfer failures as decode errors, so the `is_body()`/`is_connect()` arms are not reachable with any of the three fixtures tried. |
| S2.8 `non_exhaustive` / never-evaluated / fixture-only arms | 8 | `unreachable-by-construction` | `lib.rs` 1699 (the burst producer's early `return`: it needs the drain loop to have returned, which the fixture forbids before the burst ends), 1878–1879 (`Failed::flush`'s `Ok`: the two writer cases are "write fails" and "flush fails"); `runtime.rs` 248–252 (the frozen registry's two closure bodies — the test asserts they never run); `desktop_shutdown.rs:501` (the `std::future::pending` sentinel). Every one of these sits in **test** code, so the invariant is the fixture's own design. |
| S2.12 the startup reconciliation's failure/removal arms — an identified fixture, **not landed** | 7 | **not waived — `REWORK`** (PLAN §4) | `lib.rs` 1319 and 1327–1332 are **production** arms of `start_backend_background`'s post-readiness block (`store::reconcile_orphan_sessions` and `reconcile_empty_conversations` returning `Err`, plus the `Ok(removed)` removal path). They are not unreachable by construction: `store::db::connect()` propagates `Connection::open`/`CONNECTION_PRAGMAS` failures for a non-database `app.db`, and the launch fixture's mock agent keeps `list_agent_session_ids()` `Ok`, so a fixture that breaks the app DB after `initialize_app_store()` and then calls `start_backend_background` reaches them (the `Ok(removed)` arm additionally needs a planted empty conversation). Named here rather than waived (§S2.10 measured the over-claim). |
| S2.9 region llvm-cov attributes to a block-end brace | 3 | `attribution-artifact` | `lib.rs:1370` closes `if !enabled { return; }` (1368–1369 covered); `desktop_shutdown.rs` 263 and 460 close blocks whose bodies are covered (260–262, 457–459). |
| S2.10 downstream of the remote bridge's private state | 10 | `unreachable-in-this-environment` | `desktop_shutdown.rs` 256–258 and `lib.rs` 1372–1383 (§S2.8.3): reaching them needs `remote::SUPERVISOR.state` to be `Some`, whose only installer (`remote::tests::runtime_tests::install_state`/`fake_state`) is private to `src/remote/**` — outside this unit's write set — while the public route starts a real bridge on the port this suite already fails to release (§S2.7). |
| S2.11 an OS thread-creation refusal | 7 | `unreachable-in-this-environment` | `coordinator.rs` 165–166 and 183–187 are the two `std::thread::Builder::spawn` error arms: forcing one needs an OS refusal (thread/`RLIMIT_NPROC` exhaustion) that cannot be induced without changing a process-wide limit and breaking every sibling test in the same binary; the thread names are compile-time constants, so there is no injection point. |
| **total** | **270** | | |

Checked mechanically before hand-off:

Pass **one `--scope` per file in this unit's scope**. `--scope` is a prefix, so the
single `--scope desktop/src-tauri/src` the first draft used swept every peer unit's
file in as well and their uncovered lines were reported `MISSING` here (see S2.10).
Because `reconcile.py` treats a line starting with `# ` as a level-1 heading, a
`#`-comment inside a fenced block also truncates the section slice it checks — this
block is therefore comment-free.

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-seg2b.info \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 2" --label tauri-shell-seg2
 # → files with uncovered lines=11  uncovered lines=270
 #   mentioned in module-tauri-shell.md: 270
 #   MISSING (uncovered, not mentioned): 0        (STALE: none printed → 0)
```

### Per-line index of this segment's residual (270 rows)

One row per residual line of *this* revision, generated from
`/tmp/tauri-shell-lcov-seg2b.info` with the groups above, so any single line
can be checked without re-deriving its classification. §8 above keeps the
previous revision's numbers; this table is the authoritative one.

| line | group |
|---|---|
| `src/desktop_shutdown/coordinator.rs:50` | S2.1 |
| `src/lib.rs:209` | S2.1 |
| `src/lib.rs:210` | S2.1 |
| `src/lib.rs:211` | S2.1 |
| `src/lib.rs:212` | S2.1 |
| `src/lib.rs:213` | S2.1 |
| `src/lib.rs:214` | S2.1 |
| `src/lib.rs:215` | S2.1 |
| `src/lib.rs:474` | S2.1 |
| `src/lib.rs:475` | S2.1 |
| `src/lib.rs:476` | S2.1 |
| `src/lib.rs:477` | S2.1 |
| `src/lib.rs:478` | S2.1 |
| `src/lib.rs:811` | S2.1 |
| `src/lib.rs:842` | S2.1 |
| `src/lib.rs:888` | S2.1 |
| `src/lib.rs:889` | S2.1 |
| `src/lib.rs:890` | S2.1 |
| `src/lib.rs:907` | S2.1 |
| `src/lib.rs:908` | S2.1 |
| `src/lib.rs:909` | S2.1 |
| `src/lib.rs:916` | S2.1 |
| `src/lib.rs:919` | S2.1 |
| `src/lib.rs:920` | S2.1 |
| `src/lib.rs:921` | S2.1 |
| `src/lib.rs:932` | S2.1 |
| `src/lib.rs:933` | S2.1 |
| `src/lib.rs:934` | S2.1 |
| `src/lib.rs:935` | S2.1 |
| `src/lib.rs:936` | S2.1 |
| `src/lib.rs:937` | S2.1 |
| `src/lib.rs:941` | S2.1 |
| `src/lib.rs:942` | S2.1 |
| `src/lib.rs:943` | S2.1 |
| `src/lib.rs:945` | S2.1 |
| `src/lib.rs:950` | S2.1 |
| `src/lib.rs:955` | S2.1 |
| `src/lib.rs:956` | S2.1 |
| `src/lib.rs:957` | S2.1 |
| `src/lib.rs:958` | S2.1 |
| `src/lib.rs:960` | S2.1 |
| `src/lib.rs:962` | S2.1 |
| `src/lib.rs:963` | S2.1 |
| `src/lib.rs:964` | S2.1 |
| `src/lib.rs:965` | S2.1 |
| `src/lib.rs:967` | S2.1 |
| `src/lib.rs:968` | S2.1 |
| `src/lib.rs:969` | S2.1 |
| `src/lib.rs:970` | S2.1 |
| `src/lib.rs:971` | S2.1 |
| `src/lib.rs:974` | S2.1 |
| `src/lib.rs:975` | S2.1 |
| `src/lib.rs:980` | S2.1 |
| `src/lib.rs:984` | S2.1 |
| `src/lib.rs:985` | S2.1 |
| `src/lib.rs:986` | S2.1 |
| `src/lib.rs:991` | S2.1 |
| `src/lib.rs:992` | S2.1 |
| `src/lib.rs:993` | S2.1 |
| `src/lib.rs:995` | S2.1 |
| `src/lib.rs:1004` | S2.1 |
| `src/lib.rs:1005` | S2.1 |
| `src/lib.rs:1006` | S2.1 |
| `src/lib.rs:1007` | S2.1 |
| `src/lib.rs:1008` | S2.1 |
| `src/lib.rs:1010` | S2.1 |
| `src/lib.rs:1011` | S2.1 |
| `src/lib.rs:1031` | S2.1 |
| `src/lib.rs:1032` | S2.1 |
| `src/lib.rs:1033` | S2.1 |
| `src/lib.rs:1034` | S2.1 |
| `src/lib.rs:1035` | S2.1 |
| `src/lib.rs:1036` | S2.1 |
| `src/lib.rs:1037` | S2.1 |
| `src/lib.rs:1038` | S2.1 |
| `src/lib.rs:1039` | S2.1 |
| `src/lib.rs:1040` | S2.1 |
| `src/lib.rs:1195` | S2.1 |
| `src/lib.rs:1196` | S2.1 |
| `src/lib.rs:1202` | S2.1 |
| `src/lib.rs:1203` | S2.1 |
| `src/lib.rs:1204` | S2.1 |
| `src/lib.rs:1205` | S2.1 |
| `src/lib.rs:1206` | S2.1 |
| `src/lib.rs:1207` | S2.1 |
| `src/lib.rs:1208` | S2.1 |
| `src/lib.rs:1214` | S2.1 |
| `src/lib.rs:1220` | S2.1 |
| `src/lib.rs:1221` | S2.1 |
| `src/lib.rs:1222` | S2.1 |
| `src/lib.rs:1223` | S2.1 |
| `src/lib.rs:1224` | S2.1 |
| `src/lib.rs:1225` | S2.1 |
| `src/lib.rs:1226` | S2.1 |
| `src/lib.rs:1227` | S2.1 |
| `src/lib.rs:1228` | S2.1 |
| `src/lib.rs:1229` | S2.1 |
| `src/lib.rs:1230` | S2.1 |
| `src/lib.rs:1231` | S2.1 |
| `src/lib.rs:1232` | S2.1 |
| `src/lib.rs:1233` | S2.1 |
| `src/lib.rs:1238` | S2.1 |
| `src/lib.rs:1239` | S2.1 |
| `src/lib.rs:1241` | S2.1 |
| `src/lib.rs:1242` | S2.1 |
| `src/lib.rs:1243` | S2.1 |
| `src/lib.rs:1244` | S2.1 |
| `src/lib.rs:1246` | S2.1 |
| `src/lib.rs:1247` | S2.1 |
| `src/lib.rs:1248` | S2.1 |
| `src/lib.rs:1249` | S2.1 |
| `src/lib.rs:1250` | S2.1 |
| `src/lib.rs:1254` | S2.1 |
| `src/lib.rs:1255` | S2.1 |
| `src/lib.rs:1256` | S2.1 |
| `src/lib.rs:1257` | S2.1 |
| `src/lib.rs:1258` | S2.1 |
| `src/lib.rs:1259` | S2.1 |
| `src/lib.rs:1260` | S2.1 |
| `src/lib.rs:1261` | S2.1 |
| `src/lib.rs:1262` | S2.1 |
| `src/desktop_shutdown.rs:256` | S2.10 |
| `src/desktop_shutdown.rs:257` | S2.10 |
| `src/desktop_shutdown.rs:258` | S2.10 |
| `src/lib.rs:1372` | S2.10 |
| `src/lib.rs:1373` | S2.10 |
| `src/lib.rs:1374` | S2.10 |
| `src/lib.rs:1375` | S2.10 |
| `src/lib.rs:1377` | S2.10 |
| `src/lib.rs:1381` | S2.10 |
| `src/lib.rs:1383` | S2.10 |
| `src/desktop_shutdown/coordinator.rs:165` | S2.11 |
| `src/desktop_shutdown/coordinator.rs:166` | S2.11 |
| `src/desktop_shutdown/coordinator.rs:183` | S2.11 |
| `src/desktop_shutdown/coordinator.rs:184` | S2.11 |
| `src/desktop_shutdown/coordinator.rs:185` | S2.11 |
| `src/desktop_shutdown/coordinator.rs:186` | S2.11 |
| `src/desktop_shutdown/coordinator.rs:187` | S2.11 |
| `src/macos_power.rs:21` | S2.2 |
| `src/macos_power.rs:22` | S2.2 |
| `src/macos_power.rs:23` | S2.2 |
| `src/macos_power.rs:24` | S2.2 |
| `src/macos_power.rs:26` | S2.2 |
| `src/macos_power.rs:27` | S2.2 |
| `src/macos_power.rs:29` | S2.2 |
| `src/macos_power.rs:30` | S2.2 |
| `src/macos_power.rs:31` | S2.2 |
| `src/macos_power.rs:32` | S2.2 |
| `src/menu.rs:26` | S2.2 |
| `src/menu.rs:27` | S2.2 |
| `src/menu.rs:28` | S2.2 |
| `src/menu.rs:29` | S2.2 |
| `src/menu.rs:31` | S2.2 |
| `src/menu.rs:32` | S2.2 |
| `src/menu.rs:33` | S2.2 |
| `src/menu.rs:34` | S2.2 |
| `src/menu.rs:35` | S2.2 |
| `src/menu.rs:36` | S2.2 |
| `src/menu.rs:37` | S2.2 |
| `src/menu.rs:38` | S2.2 |
| `src/menu.rs:39` | S2.2 |
| `src/menu.rs:40` | S2.2 |
| `src/menu.rs:41` | S2.2 |
| `src/menu.rs:42` | S2.2 |
| `src/menu.rs:43` | S2.2 |
| `src/menu.rs:44` | S2.2 |
| `src/menu.rs:45` | S2.2 |
| `src/menu.rs:46` | S2.2 |
| `src/menu.rs:47` | S2.2 |
| `src/menu.rs:48` | S2.2 |
| `src/menu.rs:49` | S2.2 |
| `src/menu.rs:51` | S2.2 |
| `src/menu.rs:52` | S2.2 |
| `src/menu.rs:53` | S2.2 |
| `src/menu.rs:54` | S2.2 |
| `src/menu.rs:55` | S2.2 |
| `src/menu.rs:56` | S2.2 |
| `src/menu.rs:57` | S2.2 |
| `src/menu.rs:58` | S2.2 |
| `src/menu.rs:59` | S2.2 |
| `src/menu.rs:61` | S2.2 |
| `src/menu.rs:62` | S2.2 |
| `src/menu.rs:63` | S2.2 |
| `src/menu.rs:65` | S2.2 |
| `src/menu.rs:66` | S2.2 |
| `src/menu.rs:67` | S2.2 |
| `src/menu.rs:68` | S2.2 |
| `src/menu.rs:69` | S2.2 |
| `src/menu.rs:70` | S2.2 |
| `src/menu.rs:72` | S2.2 |
| `src/menu.rs:73` | S2.2 |
| `src/menu.rs:74` | S2.2 |
| `src/menu.rs:75` | S2.2 |
| `src/menu.rs:76` | S2.2 |
| `src/menu.rs:77` | S2.2 |
| `src/menu.rs:78` | S2.2 |
| `src/launch.rs:330` | S2.3 |
| `src/launch.rs:331` | S2.3 |
| `src/launch.rs:332` | S2.3 |
| `src/launch.rs:333` | S2.3 |
| `src/launch.rs:334` | S2.3 |
| `src/launch.rs:338` | S2.3 |
| `src/launch.rs:339` | S2.3 |
| `src/launch.rs:340` | S2.3 |
| `src/launch.rs:341` | S2.3 |
| `src/launch.rs:342` | S2.3 |
| `src/launch.rs:343` | S2.3 |
| `src/launch.rs:344` | S2.3 |
| `src/launch.rs:345` | S2.3 |
| `src/launch.rs:346` | S2.3 |
| `src/launch.rs:347` | S2.3 |
| `src/main.rs:9` | S2.3 |
| `src/main.rs:29` | S2.3 |
| `src/main.rs:33` | S2.3 |
| `src/main.rs:34` | S2.3 |
| `src/main.rs:38` | S2.3 |
| `src/main.rs:39` | S2.3 |
| `src/main.rs:41` | S2.3 |
| `src/main.rs:42` | S2.3 |
| `src/main.rs:43` | S2.3 |
| `src/main.rs:44` | S2.3 |
| `src/main.rs:46` | S2.3 |
| `src/main.rs:47` | S2.3 |
| `src/main.rs:48` | S2.3 |
| `src/main.rs:51` | S2.3 |
| `src/main.rs:58` | S2.3 |
| `src/main.rs:70` | S2.3 |
| `src/main.rs:71` | S2.3 |
| `src/main.rs:72` | S2.3 |
| `src/main.rs:73` | S2.3 |
| `src/main.rs:74` | S2.3 |
| `src/main.rs:75` | S2.3 |
| `src/bin/futureos-headless.rs:11` | S2.4 |
| `src/desktop_shutdown.rs:221` | S2.5 |
| `src/desktop_shutdown.rs:222` | S2.5 |
| `src/desktop_shutdown.rs:223` | S2.5 |
| `src/desktop_shutdown.rs:267` | S2.5 |
| `src/desktop_shutdown.rs:268` | S2.5 |
| `src/desktop_shutdown.rs:269` | S2.5 |
| `src/desktop_shutdown.rs:270` | S2.5 |
| `src/desktop_shutdown.rs:271` | S2.5 |
| `src/desktop_shutdown.rs:272` | S2.5 |
| `src/desktop_shutdown.rs:273` | S2.5 |
| `src/desktop_shutdown.rs:274` | S2.5 |
| `src/instance.rs:164` | S2.6 |
| `src/instance.rs:165` | S2.6 |
| `src/instance.rs:166` | S2.6 |
| `src/instance.rs:167` | S2.6 |
| `src/instance.rs:168` | S2.6 |
| `src/future_login.rs:354` | S2.7 |
| `src/future_login.rs:355` | S2.7 |
| `src/future_login.rs:482` | S2.7 |
| `src/desktop_shutdown.rs:501` | S2.8 |
| `src/lib.rs:1319` | S2.12 |
| `src/lib.rs:1327` | S2.12 |
| `src/lib.rs:1328` | S2.12 |
| `src/lib.rs:1329` | S2.12 |
| `src/lib.rs:1330` | S2.12 |
| `src/lib.rs:1331` | S2.12 |
| `src/lib.rs:1332` | S2.12 |
| `src/lib.rs:1699` | S2.8 |
| `src/lib.rs:1878` | S2.8 |
| `src/lib.rs:1879` | S2.8 |
| `src/runtime.rs:248` | S2.8 |
| `src/runtime.rs:249` | S2.8 |
| `src/runtime.rs:251` | S2.8 |
| `src/runtime.rs:252` | S2.8 |
| `src/desktop_shutdown.rs:263` | S2.9 |
| `src/desktop_shutdown.rs:460` | S2.9 |
| `src/lib.rs:1370` | S2.9 |

## S2.7 Required gates (PLAN §5.1) and the suite

Run in `desktop/src-tauri`, same environment as §1, on this revision:

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check --manifest-path desktop/src-tauri/Cargo.toml` | **1** | **one** diff remains, in `src/commands/update.rs` — not in this unit's write set, and the file is untouched by this unit (`cargo test`'s unit is fine; the file belongs to the commands unit). Measured twice: at 13:12 it was 21 diffs across 10 peer files, at 13:2x (after those units formatted their own files) exactly this one is left. **This unit's own files are clean** — `lib.rs`, `launch.rs`, `instance.rs`, `host_environment.rs`, `desktop_shutdown.rs`, `desktop_shutdown/coordinator.rs` were formatted per file (`rustfmt --edition 2021 --config skip_children=true <file>`, never `cargo fmt` across the crate, per PLAN §2b), and the formatter's expansion of two pre-existing one-line asserts is what moved `launch.rs`'s line numbers. |
| clippy (gui) | `cargo clippy --all-targets -- -D warnings` | **0** | clean. The one error this unit introduced (`clippy::manual_contains` in the new coordinator test's `snapshot.errors.iter().any(…)`) was fixed in this segment; an earlier run also showed 11 errors in 7 peer files (`remote/verify_e2e.rs`, `terminal/session.rs`, `skills_bootstrap.rs`, `headless/mod.rs`, `cli_tool.rs`, `terminal/pty.rs`, `remote_host/lean.rs`), which those units have since fixed. |
| clippy (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **0** | clean (an earlier run: 5 errors in `remote/verify_e2e.rs` + `remote_host/lean.rs`, since fixed by their unit). |
| tests (gui) | `cargo llvm-cov --no-report … -- --test-threads=1` (the run above) | **0** | 2005 passed / 0 failed / 12 ignored in the lib target; `futureos` bin 2/0, `futureos-headless` bin 1/0, `tests/headless_cli.rs` 9/0 |
| tests (headless) | `cargo llvm-cov --no-report … --no-default-features --features headless …` | **101** | 1597 passed / 3 failed / 12 ignored (the three named below) |
| this unit's new tests | the eight filters in §S2.9, uninstrumented, `--test-threads=1` | **0** | 1 passed / 0 failed each |

The **gui** flavour is green in the measurement this record is written against. The
same two `remote::*` tests *did* fail in this segment's earlier full run of the same
revision (gui: 2003 passed / 2 failed), which is what makes them order/state
dependent rather than a fixed defect — see below.

**The failing tests are outside this unit's scope, and one is not a flake at all in
this worktree:**

* `remote::commands::bridge_tests::tool_call_args_is_reachable_only_for_a_client_that_declared_lean`
  (`src/remote/commands.rs:2422`, `assert_eq!(reply["success"], json!(false))` → got
  `true`) and `remote::tests::runtime_tests::a_client_that_did_not_declare_keeps_the_full_lane`
  (`src/remote/tests.rs:909`, expected `thinking_delta`, got `tool_end`) failed in this
  segment's earlier full run **and** in the headless flavour here. Reproduced
  independently of this unit: `cargo test --lib -- --test-threads=1 remote::` (a
  filter that cannot match any of this unit's tests) → **FAILED. 249 passed; 2
  failed**, the same two; each passes in isolation (`… --exact
  <name>`, exit 0), so it is a cross-test state leak inside `src/remote/**` (a peer
  unit's in-flight module), not a defect of this unit.
* `remote_peer::transfer_tests::a_chunk_write_on_a_dead_socket_is_recorded` — the
  documented pre-existing host failure: `web server bind on port 8022 failed:
  Address already in use (os error 48)` → `the test web port never freed`
  (`src/remote_peer/testing.rs:30`). Out of scope; recorded, not "fixed".

Repro for the three gate commands and the suite (verbatim, from the worktree root):

```bash
cd desktop/src-tauri
cargo fmt --check ; cargo clippy --all-targets -- -D warnings
cargo clippy --no-default-features --features headless --all-targets -- -D warnings
cargo test --lib --no-fail-fast -- --test-threads=1        # 2003/2/12
cargo test --lib -- --test-threads=1 remote::              # the 2 remote failures, without this unit
cargo test --lib -- --test-threads=1 \
  a_success_after_the_deadline_is_rewritten_into_a_failure \
  the_desktop_registration_builds_for_any_runtime \
  backend_background_work_starts_for_any_runtime \
  the_terminal_delivery_prints_the_launcher_notice_to_this_stdout \
  a_host_without_a_resolvable_cli_still_starts_logging \
  a_lock_failure_that_is_not_contention_is_not_reported_as_another_instance \
  runtime_update_drain_loop_covers_every_receive_exit
```

## S2.8 Findings

### S2.8.1 A count-bounded burst is a scheduling race; a time-bounded one is not

§4.11 row 3 (`lib.rs` 833, the coalescing loop's `remaining.is_zero()` break) was
the previous segment's flakiest line: a 20-message burst sent at 2 ms intervals can
finish *before* the drain thread is scheduled on a loaded host, which is what left
the line `DA:0` while its own test passed. The fixture now sends for a wall-clock
500 ms window, so the drain thread has to be scheduled inside it. Line 833 is
covered in this measurement.

### S2.8.2 The terminal delivery is asserted at fd 1, not through `println!`

§4.11 row 4 (`lib.rs` 1384–1389) writes `launch::invitation_notice` to
`std::io::stdout()`. libtest captures `println!` but **not** the `Stdout` handle
below it, so the fixture points this process's fd 1 at a file for the duration of
the delivery, polls the file for the notice text, and restores fd 1 on drop
(`StdoutCapture`). The assertion is therefore the arm's real effect — the exact
launcher notice on this process's stdout — and it doubles as the completion signal
for work that runs on the background runtime, with no blind sleep.

### S2.8.3 The two remote-dependent rows are registered, not faked

Rows 5 (`lib.rs` 1348–1359, the auto-connect pass) and 6 (`desktop_shutdown.rs`
256–258, the remote-shutdown deadline arm) need `remote::SUPERVISOR.state` to be
Some. The only fixture that installs it (`remote::tests::runtime_tests::{fake_state,install_state}`)
is private to `src/remote/**`, outside this unit's write set; reaching it through
the public API means starting a *real* bridge (the mock platform + NATS + the web
server on 8022) — the same port whose non-release is already this suite's
documented host failure (§S2.7), and a module a peer unit is editing in this
worktree. Both rows are registered `unreachable-in-this-environment` with the
fixture named, and this segment's fixture covered the *gate* they sit behind
(`spawn_remote_auto_connect`'s three-condition check and its early return are now
covered) while leaving the spawned pass itself uncovered. A successor who owns
`src/remote/**` can close both rows; the honest arithmetic is 8 open lines here,
not a test that asserts nothing.

### S2.8.4 Two lines that flip between runs for a reason this unit's fixture cannot control

`desktop_shutdown.rs` 499–500 (the body of the future spawned by
`runtime_failure_still_drops_background_work_and_blocks_retry`) are **covered in
isolation**, `DA:0` in this segment's first full run (both flavours) and covered
again in the final run — i.e. they are not stable across runs:

```bash
cargo llvm-cov clean --manifest-path Cargo.toml
cargo llvm-cov --no-report --manifest-path Cargo.toml -- \
    runtime_failure_still_drops_background_work_and_blocks_retry --test-threads=1
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tau-c2-probe.info
python3 .future/cov100/covlines.py detail /tmp/tau-c2-probe.info \
    --under desktop/src-tauri/src/desktop_shutdown.rs     # 499 and 500 absent → covered
```

Mechanism: `runtime::spawn` drops a future un-polled when the process-global
`runtime::TASKS` registry is closed, and a *sibling fixture's* non-abortable
`perform_stop` thread can call `runtime::freeze_tasks()` between this test's own
`cancel_test_tasks()` and its `spawn`. The test passes either way — the completion
guard is armed before the first poll — so the two lines are conditionally executed.
They are **covered in the measurement this record is written against** (so they need
no category here), and the probe is kept as the reproduction because a reviewer who
sees them `DA:0` in a later run should not read it as a regression: the fixture
resets the flag immediately before spawning, and a retry loop would turn a 2-line
attribution into a flaky red test.

### S2.8.5 The `--test-threads=1` main-thread fact, re-confirmed

The mock-runtime registration test aborted with `muda::MenuChild can only be
created on the main thread` at `Builder::build` before `enable_macos_default_menu(false)`
was added: the §6.2 finding (libtest never runs a test body on the main thread,
even with `--test-threads=1`) is what makes Tauri's *own default menu* unbuildable
in a test process, not just `menu::build_macos_menu`.

## S2.9 Reproduction

```bash
 # 1. the measurement (§S2.1): bash /tmp/tau-c2-measure.sh   (~25 min, both flavours)
 # 2. this unit's new tests, uninstrumented and fast:
cd desktop/src-tauri
cargo test --lib -- --test-threads=1 runtime_update_drain_loop_covers_every_receive_exit
cargo test --lib -- --test-threads=1 the_terminal_delivery_prints_the_launcher_notice_to_this_stdout
cargo test --lib -- --test-threads=1 the_desktop_registration_builds_for_any_runtime
cargo test --lib -- --test-threads=1 backend_background_work_starts_for_any_runtime
cargo test --lib -- --test-threads=1 a_host_without_a_resolvable_cli_still_starts_logging
cargo test --lib -- --test-threads=1 a_success_after_the_deadline_is_rewritten_into_a_failure
cargo test --lib -- --test-threads=1 a_lock_failure_that_is_not_contention_is_not_reported_as_another_instance
cargo test --lib -- --test-threads=1 unconfirmed_runs_report_the_transport_outcome
 # 3. the isolation probe for §S2.8.4 (its lcov is /tmp/tau-c2-probe.info)
 # 4. the gate commands in §S2.7
```

## S2.10 Re-verification on the retry (gates re-run, arithmetic re-checked)

Nothing in S2.1–S2.9 is restated below; this subsection re-derives the numbers from
the same report and re-runs the gates PLAN §5.1 requires, after the acceptance rule
was tightened to "a green suite is not a green PR — run `fmt` and `clippy` **and**
report their exit codes".

**The report describes the code on disk.** This unit's scope files were last written
at `20:33:30` and the merged run that produced `/tmp/tauri-shell-lcov-seg2b.info`
started at `20:35:23` and finished at `20:54:44` local (`12:35:23 → 12:54:44` UTC,
`/tmp/tau-c2-measure3.log`); `git status` showed the unit's paths clean then, in the
snapshot that followed (`2c9aa504` — HEAD has since moved under the supervisor's own
commits, and §S2.6's arithmetic below is re-derived from the *working tree*, whose
scope files are still byte-identical to the measured ones). No scope file has been
touched since, so the segment's numbers and the working tree are the same revision.
Peer-owned files
(`scheduler/mod.rs`, `commands/*.rs`, `remote/*`, `remote_host/*`, `headless/*`) were
written after `20:35` — their own records', not this one's, problem.

### Gates, re-run verbatim (2026-10-16 21:2x local, `desktop/src-tauri`, own target dir)

| gate | command | exit | result |
|---|---|---|---|
| format (crate) | `cargo fmt --check` | **1** | **one** diff, `src/commands/update.rs:1145` — a peer unit's file, edited minutes earlier (unformatted `let _home = …` rubric line inside `a_restart_is_refused_without_a_safe_record`). Not in this unit's write set; §S2.7 measured the same single diff at 13:12 UTC |
| format (this unit) | `rustfmt --edition 2021 --config skip_children=true --check <file>` for all 12 scope files | **0** each | clean; the crate-wide red is not this unit's |
| clippy (gui) | `cargo clippy --all-targets -- -D warnings` | **0** | clean, 9.7 s |
| clippy (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **0** | clean, 13.2 s |
| the 8 fixtures | `cargo test --lib --no-fail-fast -- --test-threads=1 <the 8 names>` | **0** | `8 passed; 0 failed; 0 ignored; 2012 filtered out` |
| lib suite, gui, uninstrumented | `cargo test --lib --no-fail-fast -- --test-threads=1` | **101** | `2002 passed; 6 failed; 12 ignored` — all six outside this unit, named below |
| lib suite, headless, uninstrumented | `cargo test --no-default-features --features headless --lib --no-fail-fast -- --test-threads=1` | **0** | `1604 passed; 0 failed; 12 ignored` — **green**, including the two `headless::tests::*` cases that failed in the gui-flavor run eight minutes earlier, which is the load-dependence below, measured |
| lib suite, gui, instrumented (the measurement) | §S2.1 gui leg | **0** | `2005 passed; 0 failed; 12 ignored` |
| lib suite, headless, instrumented (the measurement) | §S2.1 headless leg | **101** | `1597 passed; 3 failed; 12 ignored` — all three outside this unit, named below |

The six failures in the uninstrumented re-run, and why none of them is this unit's:

* `remote_peer::runtime_tests::connect_all_leaves_a_live_host_on_its_own_connection`,
  `…::connect_all_resumes_a_host_the_user_had_stopped`,
  `…::connecting_an_unknown_host_is_an_error_not_an_invention`,
  `…::connecting_replaces_an_existing_retry_loop` — all four panic at
  `src/remote_peer/testing.rs:30` (`the test web port never freed`) after
  `web server bind on port 8022 failed: Address already in use (os error 48)`; the
  measurement run's headless leg failed three of the same class
  (`remote_peer::runtime::tests::*`). Cause visible on the host: **three** peer
  `cargo test` processes of this same crate were live in this worktree at the time
  (`ps`: `cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib`, `cargo test --lib`,
  and a `future-agent` llvm-cov leg), and port 8022 is a fixed test port — the first
  failure to bind leaves the port held and the rest cascade. Pre-existing host
  failure, out of this unit's scope; no fixture of this unit touches
  `remote_peer`.
* `headless::tests::the_builtin_skill_bootstrap_runs_the_cli` and
  `headless::tests::a_failing_builtin_skill_bootstrap_is_ignored` — both fail with
  `CLI discovery timed out … no fallback CLI was executed` against their own fixture
  CLI. The files involved (`src/headless/mod.rs`, `src/cli_tool.rs`,
  `skills_bootstrap.rs`) are peer-owned and were written at `19:59–20:00`, i.e. in the
  middle of this campaign; `cli_tool.rs` is the file §6.7 already flagged as rewritten
  under `host_environment.rs:28`. They are not in this unit's write set and this unit's
  fixtures do not call into `headless::tests`. Both are also load-sensitive (a discovery
  timeout, with three peer suites on the same host) — and **measured**: the same two
  tests pass in the headless-flavour run of this tree taken eight minutes later
  (`1604 passed; 0 failed`, row above). Reported, not papered over: this pair is the one
  thing in the re-run a reviewer should look at first, and it is the host's load plus a
  peer unit's in-flight `cli_tool`/`headless` state, not this segment's seam.

### Arithmetic, re-derived line by line

* per-file `DA:0` from `covlines.py total … --under <file>` for the 12 scope files:
  `138 + 48 + 15 + 21 + 17 + 3 + 8 + 10 + 4 + 5 + 0 + 1 = 270` — identical to §S2.2;
* **both** before columns re-derived from their own reports, not copied: the
  `8449acf6` baseline (`.future/cov100/baseline/tauri-merged-lcov.info`) is
  `203 + 48 + 38 + 28 + 20 + 18 + 15 + 10 + 8 + 7 + 0 + 5 = 400`, the previous segment
  (`/tmp/tauri-shell-lcov-final.info`) is `181 + 48 + 15 + 21 + 18 + 3 + 9 + 10 + 4 + 7 + 1 + 1 = 318`,
  and this segment is `270` — §2's two columns and §S2.2's left column all reproduce;
* the OPEN-row arithmetic of §S2.3 closes: of the 29 `§4.11` rows, **12 lines are
  covered** (rows 1–4, 7, 9 = `2+1+1+6+1+1`) and **17 are registered**
  (rows 5, 6, 8 = `7+3+7`), so the previous segment's `289 waived + 29 open` becomes
  **`246 waived + 17 registered + 7 rework`**, and `318 − 48 = 270`;
* **a category correction this turn found** (§S2.12): the first draft filed `lib.rs`
  1319 and 1327–1332 under S2.8's `unreachable-by-construction`, arguing that the launch
  fixture's mock agent answers successfully. That is a statement about the fixture, not
  an invariant: `store::db::connect()` returns `Err` for a non-database `app.db`
  (`Connection::open` then `CONNECTION_PRAGMAS`), `db_path()` and `ensure_app_dirs()`
  propagate too, and the block runs 3 s after the launch path — so a broken-DB fixture
  reaches the arms. The seven lines (all **production** code, unlike the rest of S2.8)
  are therefore re-filed as `S2.12` and left **un-waived**; the 270, the per-line index
  and this measurement are unchanged, only the category is honest now. S2.8 has 8 lines
  left, every one of them in test code, where "the fixture's own design" *is* the
  invariant. This is the §4 bar the reviewer hunts for: a wrong waiver is worse than an
  open row.
* §S2.6's group counts sum to **270**, and §S2.6's per-line index has **270** rows,
  matching the report's `DA:0` set for the scope (checked against the tool's per-file
  lists, not by eye);
* `reconcile.py` (12 `--scope` prefixes, §S2.6): `uncovered lines=270`,
  `MISSING: 0`, and the STALE half is **non-vacuous** — the `Segment 2` slice parses
  **270** ledger claims (the per-line index), `STALE: 0`, so the run prints no
  warning and `exit 0`. Note for whoever edits this file next: `reconcile.py` reads a
  line starting with `# ` as a level-1 heading, so a `#`-comment inside a fenced block
  silently truncates the section slice — it made this exact check read
  `270 mentioned / MISSING 213` in one intermediate draft, and the block in §S2.6 is
  comment-free for that reason;
* spot values straight out of the report, as evidence the classifications match it:
  `lib.rs` 833=**14** (the time-bounded burst), 901/902=**1** (`Builder::<R>::new`,
  single-instance plugin), 925–927=**1** (dialog/shell/updater + menu hook),
  1268=**2** (`start_backend_background`), 1345/1349=**2**, 1352=**1** (the seam's
  `spawn_native`, re-pair branch and calls), 1384=**2** (the fd-1 notice);
  `host_environment.rs:28`=**2**, `instance.rs` 196/197=**2**,
  `desktop_shutdown.rs` 589=**2**, `desktop_shutdown/coordinator.rs:61`=**2** — those are
  the non-`lib.rs` lines §4.11 left open (rows 1, 2, 7, 9 = 5 lines) together with row 3
  (`lib.rs:833`); §4.11's rows 1–4, 7 and 9 account for the 12 newly covered row lines;
  and the registered zeros: `lib.rs` 1372–1383, `desktop_shutdown.rs` 256–258,
  `coordinator.rs` 165–166/183–187 = **0**; `lib.rs:1370` = **0** (the block-end
  brace, `attribution-artifact`, with 1368–1369 = 2).

### Host-state candidates: which ones the measurement could resolve

§6.7 and the supervisor's note ask for the class of line whose arm is decided by host
state (`PATH`, a live port, a scheduling window) to be pinned or registered, and for
the resolved candidates to be named:

| candidate | state in this report | disposition |
|---|---|---|
| `host_environment.rs:28` (`report(&error)` when `cli_tool::resolve()` fails) | **covered** (=2), was `DA:0` at the previous revision | pinned by this unit's fixture `a_host_without_a_resolvable_cli_still_starts_logging`, which calls `start_logging()` with a PATH where the CLI cannot resolve — the fixture §4.11/§6.7 named. It no longer depends on the ambient PATH |
| `instance.rs` 196–197 | **covered** (=2) | hoist (§S2.5) — no host state involved |
| `desktop_shutdown.rs` 589 (transport outcome) | **covered** (=2) | hoist (§S2.5) |
| `coordinator.rs:61` (deadline rewrite) | **covered** (=2) | reached by construction (§S2.4) |
| `lib.rs:833` (coalescing break) | **covered** (=14) | time-bounded burst, not a scheduling race (§S2.8.1) |
| `lib.rs` 1384–1389 (terminal delivery) | **covered** (=2) | fd-1 capture (§S2.8.2) |
| `desktop_shutdown.rs` 499–500 | **covered** (=1) here, `DA:0` in this segment's first full run | **not resolvable by this measurement**: the arm needs the process-global `runtime::TASKS` registry open at the instant the test spawns; §S2.8.4 keeps the isolation probe as the reproduction |
| `lib.rs` 1372–1383, `desktop_shutdown.rs` 256–258 (remote pass), `coordinator.rs` 165–166/183–187 (thread-spawn refusal) | **`DA:0`** | registered `unreachable-in-this-environment` with the fixture named (§S2.6 groups S2.10/S2.11, §S2.8.3) — pinned by fixture ownership and by an OS refusal, both outside this unit's write set |

The pattern to carry forward: a line is *pinned* when a fixture of this unit decides
the arm (rows 1–6), and *registered* when the deciding state is a peer module's private
fixture or an OS-level condition (last row). No row is left as a bare ±1.

**The MENU refactor is still rejected** (§S2.4, §S2.8.5): this retry produced no new
evidence about `muda` — no menu test was attempted, and the only main-thread datum is
the one already recorded (§6.2/§S2.8.5). If a successor finds a way to run a libtest
body on the main thread, §5.3 becomes worth re-opening; nothing so far says otherwise.

# Segment 3 — the post-build startup sequence and the run-event dispatch are extracted; 270 → 255

Status: **measured, not at target** — **255** uncovered lines in this unit's scope,
down from **270** at the end of Segment 2 and **400** at `8449acf6`. Of the 255,
**247 are waived under one of the ledger's four categories with a named mechanism**
(§S3.4) and **8 are left open, un-waived**: 7 are §S2.12's production arms that a
broken-`app.db` fixture reaches, and 1 is a fixture-only arm whose fixture is named
(§S3.6.4). Sections 1–8 and Segment 2 keep their own revisions' line numbers; this
section is authoritative for the revision measured here
(`/tmp/tauri-shell-lcov-seg3b.info`).

What changed, in one paragraph: the two seams the supervisor approved under PLAN §4b —
`initialize_process_state<R, M>` (a pure move of `run_with`'s post-build sequence) and
`dispatch_run_event<R>` (the `RunEvent` match, a pure move out of the `app.run`
closure) — were landed, with three fixtures; three self-inflicted `DA:0` lines the new
tests had introduced were hoisted (§S3.6.2); and the match's `_ => {}` arm was covered
by dispatching an event this build has no arm for (§S3.6.3). No guard was deleted or
widened, no assertion weakened, nothing `#[ignore]`d. Files touched: `src/lib.rs`
(production seams + tests) and this record. `gui_builder`/`start_backend_background`
were already landed in Segment 2 and were not touched again (§S3.3).

## S3.1 Run identity and the exact measurement

macOS arm64 (Darwin 25.x), Rust 1.97.0, `--test-threads=1` (mandatory on this host),
both flavours, one merged report — the §1 recipe (`TAURI.md`), re-run on this revision:

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-shell
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder   # exit 0

cargo llvm-cov clean --manifest-path Cargo.toml                                    # exit 0
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
 #   → exit 101: 2028 passed / 1 failed / 12 ignored
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml \
    --no-default-features --features headless -- --test-threads=1
 #   → exit 101: 1617 passed / 7 failed / 12 ignored
cargo llvm-cov report --manifest-path Cargo.toml --lcov \
    --output-path /tmp/tauri-shell-lcov-seg3b.info                                # exit 0
```

The run is the script `/tmp/tau-c3b-measure2.sh` (log `/tmp/tau-c3b-measure2.log`,
14:25:56→14:50:38 UTC); every failure is named and classified in §S3.5 and **none is
in this unit's scope**. Work list:

```bash
python3 .future/cov100/covlines.py detail /tmp/tauri-shell-lcov-seg3b.info \
    --under desktop/src-tauri/src/lib.rs
python3 .future/cov100/covlines.py total  /tmp/tauri-shell-lcov-seg3b.info \
    --under desktop/src-tauri/src
```

**Why there are two artefacts for this revision.** A first full measurement of the
revision (`/tmp/tauri-shell-lcov-seg3.info`, `/tmp/tau-c3b-measure.log`, gui exit 0 /
2027 passed, headless exit 101 / 26 failed) rendered **257** lines — but its own report
showed three `DA:0` lines the new tests had just introduced (`lib.rs` 2114, 2132, 2140,
the format arguments of three multi-line `assert!`s — precisely the trap §6.6 records).
Hoisting them is a source change, so that artefact no longer describes the tree and the
measurement was re-taken: `seg3b` is the record, and its reconciliation against the
first artefact is in §S3.6.2 (`+4` covered, `−2` for an unrelated flip). Two artefacts
exist deliberately; the number in this section is `seg3b`'s.

**The tree moved under the measurement.** `gui` reported 2027 passing tests at 14:00 and
2028 at 14:30 on the same unit's scope — a peer unit added tests to this worktree
between the two runs (`git log` shows peer snapshot commits in the interval). The
per-file figures in §S3.2 are this unit's scope only; no peer test can cover it except
by calling its code, and any such movement is called out where it appears.

## S3.2 Before and after, per file

Uncovered lines (`DA:0`), each column re-derived from its own report with
`covlines.py total … --under <file>` (never by eye, never from the `summary` block):

| file | `8449acf6` | Segment 2 | **this segment** |
|---|---|---|---|
| `src/lib.rs` | 203 | 138 | **121** |
| `src/menu.rs` | 48 | 48 | 48 |
| `src/launch.rs` | 38 | 15 | 15 |
| `src/main.rs` | 28 | 21 | 21 |
| `src/desktop_shutdown.rs` | 20 | 17 | **19** |
| `src/future_login.rs` | 18 | 3 | 3 |
| `src/desktop_shutdown/coordinator.rs` | 15 | 8 | 8 |
| `src/macos_power.rs` | 10 | 10 | 10 |
| `src/runtime.rs` | 8 | 4 | 4 |
| `src/instance.rs` | 7 | 5 | 5 |
| `src/host_environment.rs` | 0 | 0 | 0 |
| `src/bin/futureos-headless.rs` | 5 | 1 | 1 |
| **total** | **400** | **270** | **255** |

Measured (non-zero) lines in the same scope: **3602 → 3687** (`lib.rs` 886 → 971: the
two seams add the function bodies and the tests). The `desktop_shutdown.rs` column moves
the *wrong* way (17 → 19), and it is not this segment's code: `499–500` — the body of the
future `runtime_failure_still_drops_background_work_and_blocks_retry` spawns — are
**covered** in the Segment 2 report and `DA:0` here, the flip §S2.8.4 already documented
(with the isolation probe that shows them covered in isolation: a sibling
`perform_stop` thread can call `runtime::freeze_tasks()` between that test's
`cancel_test_tasks()` and its `spawn`). Both readings are real; the file is otherwise
untouched by this segment.

`lib.rs` **138 → 121 = −17**. Mechanically, from the two reports: the startup statements
Segment 2 recorded as unreachable are gone from the residual and their new lines carry
non-zero counters (§S3.3); against that, the single-instance plugin callback body
(`lib.rs` 906–924, the second-instance arm) went from **7 `DA:0` lines to 12** with no
code or test change — the attribution instability §S2.4 measured on the same block, in
the other direction (§S3.6.5). No other file moved.

## S3.3 The two seams (PLAN §4b) — what landed, and what it was worth

**(1) `initialize_process_state<R: tauri::Runtime, M: tauri::Manager<R>>(app: &M) -> Result<(), AppError>`**
— the pure move the supervisor specified (S-A1, not the "takes no `App`" variant, which
covers one line less and needs a reorder argument). Behaviour-identical: the statement
sequence, order, `&app` argument and `?`s are preserved verbatim; the guard is created
and handed to `manage` in the same statement, so its lifetime and `Drop` point are
unchanged; `run_with` is the only production caller and passes `App<Wry>`, which
implements `Manager<Wry>`. **Measured**: `lib.rs` 1222 (`acquire`), 1223 (`manage`), 1224
(`start_logging`), 1225 (`initialize_app_store`), 1226 (the `bind` `if let`) and 1229
(`Ok(())`) each carry **1**; `1227` (the `bind`-failure `eprintln!`) is **0** and stays
residual — `terminal::server::bind()` cannot be made to fail in-process (it memoises an
ephemeral-port listener, `terminal/server.rs:79-108`). The one line the move added,
`1198` (`initialize_process_state(&app)?;`), is itself inside `run_with`'s body and is
therefore `DA:0` like the rest of that body. Net for this seam: **−5.**

**(2) `dispatch_run_event<R: tauri::Runtime>(&AppHandle<R>, RunEvent, LaunchOptions)`**
— the match moved out of `app.run`'s closure, which is now
`app.run(move |app_handle, event| dispatch_run_event(app_handle, event, launch));`.
Behaviour-identical: the arms, their order and the `_ => {}` wildcard are verbatim;
`launch` was already moved into the closure and `LaunchOptions` is `Copy`; every
handle-consuming callee already takes `AppHandle<R>` (`agent_supervisor.rs:23/157/188`,
`start_backend_background`), so nothing else changed. **Measured**: `lib.rs` 1238/1243
(the function and its `match`) **3** (the three dispatches below), 1249–1251, 1255, 1258,
1260 **1** each (the `Ready` arm through supervision, the `is_stopping` gate, the launch
path, the `serve` call), 1283–1286 and 1288 **1** (the `Exit` arm and the wildcard). The
arms that stay residual are named in §S3.4 and §S3.6.4.

**`ExitRequested` stays unreachable, from the source, not from a comment.** `tauri
2.11.5`'s `ExitRequestApi` has private fields (`tx`, `code` — `src/app.rs:81-84`), its
only construction site is Tauri's own event loop (`src/app.rs:2554`), and its only public
method takes `&self`. No test outside the Tauri crate can build that variant, so the
extraction moves the 11 lines (1267–1279) without covering them.

**(3) `gui_builder<R>` / `start_backend_background<R>`**: already landed in Segment 2 and
left untouched here — the report shows them executing (`lib.rs` 901, 902, 925–927, 1268,
1345, 1349, 1352 non-zero). Nothing in this segment's write set re-opened them.

## S3.4 The residual (255 lines) — categories

| group | lines | category | what places the line there |
|---|---|---|---|
| S3.1 the real Tauri/Wry application — `run_with`'s own body (1195, 1196, 1198, 1204–1206, 888–889), the plugin/menu/window hook bodies (906–924, 932–975), the window-geometry helpers (209–215, 474–478), the runtime-update emitters (811, 842, 1004–1040), `coordinator.rs:50` | 103 | `unreachable-in-this-environment` | `Builder::<Wry>::build` creates the Wry event loop and Tauri's default macOS menu, and `muda` only builds menus on the main thread (§6.2: a libtest body is never the main thread). The hook *bodies* additionally need real window/menu/IPC events: 906–924 is the second-instance callback, 932–975 the menu and window-event handlers. The 906–924 block is also attribution-unstable between runs (§S3.6.5) |
| S3.2 macOS main-thread-only — `menu.rs` (48) + `macos_power.rs` (10) | 58 | `unreachable-in-this-environment` | `build_macos_menu` constructs `muda` items and asserts `NSThread::isMainThread`; the power hooks are AppKit/IOKit notifications. Unchanged from §4.2/§4.3 |
| S3.3 the launcher *process* bodies — `launch.rs` 330–347, all of `main.rs`'s remaining arms | 36 | `unreachable-in-this-environment` | these bodies spawn a real GUI Desktop and wait for its answer; §4.4's waiver, unchanged |
| S3.4 the Linux-only helper success arm | 1 | `platform-unmeasured` | `bin/futureos-headless.rs:11` compiles only on Linux |
| S3.5 `cfg(not(test))`-excluded blocks — `desktop_shutdown.rs` 221–223, 267–274 | 11 | `unreachable-by-construction` | both sit behind `#[cfg(all(feature = "gui", not(test)))]`; every test build removes them. Invariant: the test cfg |
| S3.6 the `flock` re-acquire window | 5 | `unreachable-in-this-environment` | `instance.rs` 164–168 needs a lock taken *inside* the re-acquire window. Unchanged from §4.7 |
| S3.7 client-stack arms | 3 | `unreachable-in-this-environment` | `future_login.rs` 354, 355, 482: §6.5's measured result — this host's stack reports body-transfer failures as decode errors, so the `is_body()`/`is_connect()` arms are not reachable with any of the three fixtures tried |
| S3.8 never-evaluated and fixture-only arms | 10 | `unreachable-by-construction` | `lib.rs` 1727 (the burst producer's early `return`: it needs the drain loop to have returned, which the fixture forbids before the burst ends), 1906–1907 (`Failed::flush`'s `Ok`: the two writer cases are "write fails" and "flush fails"); `runtime.rs` 248, 249, 251, 252 (the frozen registry's two closure bodies — the test asserts they never run); `desktop_shutdown.rs:501` (the `std::future::pending` sentinel). Every one sits in **test** code, where the fixture's own design *is* the invariant |
| S3.9 region llvm-cov attributes to a block-end brace | 3 | `attribution-artifact` | `lib.rs:1398` closes `if !enabled { return; }` (1396–1397 covered, and the arm itself is `DA:0` for the remote reason — see S3.10); `desktop_shutdown.rs` 263 and 460 close blocks whose bodies are covered (260–262, 457–459) |
| S3.10 downstream of the remote bridge's private state | 10 | `unreachable-in-this-environment` | `lib.rs` 1400–1411 (the spawned auto-connect pass) and `desktop_shutdown.rs` 256–258 (the remote-shutdown deadline arm): reaching them needs `remote::SUPERVISOR.state` to be `Some`, whose only installer (`remote::tests::runtime_tests::install_state`/`fake_state`) is private to `src/remote/**` — outside this unit's write set. The gate they sit behind is covered; the pass is not (§S2.8.3, handoff `todo_2824cf77774d`) |
| S3.11 an OS thread-creation refusal | 7 | `unreachable-in-this-environment` | `coordinator.rs` 165–166 and 183–187 are the two `std::thread::Builder::spawn` error arms: forcing one needs an OS refusal the suite cannot induce without changing a process-wide limit; the thread names are compile-time constants, so there is no injection point |
| S3.12 the `serve`-failure arm of the `Ready` path | 1 | **not waived — fixture named, deliberately un-landed** | `lib.rs:1261` needs `terminal::server::serve()` to fail, which in this process means "the listener is already taken" — and the same process cannot also observe the production direction (`1260` succeeding, §S3.6.4). A child-process fixture (the shape `commands::terminal::tests` already uses for its pre-bind arm) reaches it; landing it would invalidate the revision this record measures |
| S3.13 the startup reconciliation's failure/removal arms | 7 | **not waived — `REWORK`** (PLAN §4) | `lib.rs` 1347 and 1355–1360: `store::reconcile_orphan_sessions` and `reconcile_empty_conversations` returning `Err`, plus the `Ok(removed)` removal path. Not unreachable: `store::db::connect()` propagates `Connection::open`/`CONNECTION_PRAGMAS` failures for a non-database `app.db`, and the launch fixture's mock agent keeps `list_agent_session_ids()` `Ok` — a fixture that breaks the app DB after `initialize_app_store()` and plants one empty conversation reaches all seven (§S2.10 measured the over-claim) |
| **total** | **255** | | |

**Arithmetic**: 222 `unreachable-in-this-environment` (S3.1 103 + S3.2 58 + S3.3 36 +
S3.6 5 + S3.7 3 + S3.10 10 + S3.11 7) + 21 `unreachable-by-construction` (S3.5 11 +
S3.8 10) + 1 `platform-unmeasured` (S3.4) + 3 `attribution-artifact` (S3.9) = **247
waived**, plus **8 un-waived** (S3.12 1 + S3.13 7). `247 + 8 = 255`, and the per-line
index below has one row per line, summing to the same 255.

Checked mechanically before hand-off (PLAN §5.2) — see §S3.5 for the invocation and its
exit code.

### Per-line index of this segment's residual (255 rows)

One row per residual line of this revision, generated from
`/tmp/tauri-shell-lcov-seg3b.info` with the groups above. §8 and §S2.6 keep their own
revisions' rows.

| line | group |
|---|---|
| `src/lib.rs:209` | S3.1 |
| `src/lib.rs:210` | S3.1 |
| `src/lib.rs:211` | S3.1 |
| `src/lib.rs:212` | S3.1 |
| `src/lib.rs:213` | S3.1 |
| `src/lib.rs:214` | S3.1 |
| `src/lib.rs:215` | S3.1 |
| `src/lib.rs:474` | S3.1 |
| `src/lib.rs:475` | S3.1 |
| `src/lib.rs:476` | S3.1 |
| `src/lib.rs:477` | S3.1 |
| `src/lib.rs:478` | S3.1 |
| `src/lib.rs:811` | S3.1 |
| `src/lib.rs:842` | S3.1 |
| `src/lib.rs:888` | S3.1 |
| `src/lib.rs:889` | S3.1 |
| `src/lib.rs:890` | S3.1 |
| `src/lib.rs:906` | S3.1 |
| `src/lib.rs:907` | S3.1 |
| `src/lib.rs:908` | S3.1 |
| `src/lib.rs:909` | S3.1 |
| `src/lib.rs:910` | S3.1 |
| `src/lib.rs:916` | S3.1 |
| `src/lib.rs:917` | S3.1 |
| `src/lib.rs:919` | S3.1 |
| `src/lib.rs:920` | S3.1 |
| `src/lib.rs:921` | S3.1 |
| `src/lib.rs:923` | S3.1 |
| `src/lib.rs:924` | S3.1 |
| `src/lib.rs:932` | S3.1 |
| `src/lib.rs:933` | S3.1 |
| `src/lib.rs:934` | S3.1 |
| `src/lib.rs:935` | S3.1 |
| `src/lib.rs:936` | S3.1 |
| `src/lib.rs:937` | S3.1 |
| `src/lib.rs:941` | S3.1 |
| `src/lib.rs:942` | S3.1 |
| `src/lib.rs:943` | S3.1 |
| `src/lib.rs:945` | S3.1 |
| `src/lib.rs:950` | S3.1 |
| `src/lib.rs:955` | S3.1 |
| `src/lib.rs:956` | S3.1 |
| `src/lib.rs:957` | S3.1 |
| `src/lib.rs:958` | S3.1 |
| `src/lib.rs:960` | S3.1 |
| `src/lib.rs:962` | S3.1 |
| `src/lib.rs:963` | S3.1 |
| `src/lib.rs:964` | S3.1 |
| `src/lib.rs:965` | S3.1 |
| `src/lib.rs:967` | S3.1 |
| `src/lib.rs:968` | S3.1 |
| `src/lib.rs:969` | S3.1 |
| `src/lib.rs:970` | S3.1 |
| `src/lib.rs:971` | S3.1 |
| `src/lib.rs:974` | S3.1 |
| `src/lib.rs:975` | S3.1 |
| `src/lib.rs:980` | S3.1 |
| `src/lib.rs:984` | S3.1 |
| `src/lib.rs:985` | S3.1 |
| `src/lib.rs:986` | S3.1 |
| `src/lib.rs:991` | S3.1 |
| `src/lib.rs:992` | S3.1 |
| `src/lib.rs:993` | S3.1 |
| `src/lib.rs:995` | S3.1 |
| `src/lib.rs:1004` | S3.1 |
| `src/lib.rs:1005` | S3.1 |
| `src/lib.rs:1006` | S3.1 |
| `src/lib.rs:1007` | S3.1 |
| `src/lib.rs:1008` | S3.1 |
| `src/lib.rs:1010` | S3.1 |
| `src/lib.rs:1011` | S3.1 |
| `src/lib.rs:1031` | S3.1 |
| `src/lib.rs:1032` | S3.1 |
| `src/lib.rs:1033` | S3.1 |
| `src/lib.rs:1034` | S3.1 |
| `src/lib.rs:1035` | S3.1 |
| `src/lib.rs:1036` | S3.1 |
| `src/lib.rs:1037` | S3.1 |
| `src/lib.rs:1038` | S3.1 |
| `src/lib.rs:1039` | S3.1 |
| `src/lib.rs:1040` | S3.1 |
| `src/lib.rs:1195` | S3.1 |
| `src/lib.rs:1196` | S3.1 |
| `src/lib.rs:1198` | S3.1 |
| `src/lib.rs:1204` | S3.1 |
| `src/lib.rs:1205` | S3.1 |
| `src/lib.rs:1206` | S3.1 |
| `src/lib.rs:1227` | S3.1 |
| `src/lib.rs:1252` | S3.1 |
| `src/lib.rs:1253` | S3.1 |
| `src/lib.rs:1256` | S3.1 |
| `src/lib.rs:1261` | S3.12 |
| `src/lib.rs:1267` | S3.1 |
| `src/lib.rs:1268` | S3.1 |
| `src/lib.rs:1270` | S3.1 |
| `src/lib.rs:1271` | S3.1 |
| `src/lib.rs:1272` | S3.1 |
| `src/lib.rs:1273` | S3.1 |
| `src/lib.rs:1275` | S3.1 |
| `src/lib.rs:1276` | S3.1 |
| `src/lib.rs:1277` | S3.1 |
| `src/lib.rs:1278` | S3.1 |
| `src/lib.rs:1279` | S3.1 |
| `src/lib.rs:1347` | S3.13 |
| `src/lib.rs:1355` | S3.13 |
| `src/lib.rs:1356` | S3.13 |
| `src/lib.rs:1357` | S3.13 |
| `src/lib.rs:1358` | S3.13 |
| `src/lib.rs:1359` | S3.13 |
| `src/lib.rs:1360` | S3.13 |
| `src/lib.rs:1398` | S3.9 |
| `src/lib.rs:1400` | S3.10 |
| `src/lib.rs:1401` | S3.10 |
| `src/lib.rs:1402` | S3.10 |
| `src/lib.rs:1403` | S3.10 |
| `src/lib.rs:1405` | S3.10 |
| `src/lib.rs:1409` | S3.10 |
| `src/lib.rs:1411` | S3.10 |
| `src/lib.rs:1727` | S3.8 |
| `src/lib.rs:1906` | S3.8 |
| `src/lib.rs:1907` | S3.8 |
| `src/menu.rs:26` | S3.2 |
| `src/menu.rs:27` | S3.2 |
| `src/menu.rs:28` | S3.2 |
| `src/menu.rs:29` | S3.2 |
| `src/menu.rs:31` | S3.2 |
| `src/menu.rs:32` | S3.2 |
| `src/menu.rs:33` | S3.2 |
| `src/menu.rs:34` | S3.2 |
| `src/menu.rs:35` | S3.2 |
| `src/menu.rs:36` | S3.2 |
| `src/menu.rs:37` | S3.2 |
| `src/menu.rs:38` | S3.2 |
| `src/menu.rs:39` | S3.2 |
| `src/menu.rs:40` | S3.2 |
| `src/menu.rs:41` | S3.2 |
| `src/menu.rs:42` | S3.2 |
| `src/menu.rs:43` | S3.2 |
| `src/menu.rs:44` | S3.2 |
| `src/menu.rs:45` | S3.2 |
| `src/menu.rs:46` | S3.2 |
| `src/menu.rs:47` | S3.2 |
| `src/menu.rs:48` | S3.2 |
| `src/menu.rs:49` | S3.2 |
| `src/menu.rs:51` | S3.2 |
| `src/menu.rs:52` | S3.2 |
| `src/menu.rs:53` | S3.2 |
| `src/menu.rs:54` | S3.2 |
| `src/menu.rs:55` | S3.2 |
| `src/menu.rs:56` | S3.2 |
| `src/menu.rs:57` | S3.2 |
| `src/menu.rs:58` | S3.2 |
| `src/menu.rs:59` | S3.2 |
| `src/menu.rs:61` | S3.2 |
| `src/menu.rs:62` | S3.2 |
| `src/menu.rs:63` | S3.2 |
| `src/menu.rs:65` | S3.2 |
| `src/menu.rs:66` | S3.2 |
| `src/menu.rs:67` | S3.2 |
| `src/menu.rs:68` | S3.2 |
| `src/menu.rs:69` | S3.2 |
| `src/menu.rs:70` | S3.2 |
| `src/menu.rs:72` | S3.2 |
| `src/menu.rs:73` | S3.2 |
| `src/menu.rs:74` | S3.2 |
| `src/menu.rs:75` | S3.2 |
| `src/menu.rs:76` | S3.2 |
| `src/menu.rs:77` | S3.2 |
| `src/menu.rs:78` | S3.2 |
| `src/launch.rs:330` | S3.3 |
| `src/launch.rs:331` | S3.3 |
| `src/launch.rs:332` | S3.3 |
| `src/launch.rs:333` | S3.3 |
| `src/launch.rs:334` | S3.3 |
| `src/launch.rs:338` | S3.3 |
| `src/launch.rs:339` | S3.3 |
| `src/launch.rs:340` | S3.3 |
| `src/launch.rs:341` | S3.3 |
| `src/launch.rs:342` | S3.3 |
| `src/launch.rs:343` | S3.3 |
| `src/launch.rs:344` | S3.3 |
| `src/launch.rs:345` | S3.3 |
| `src/launch.rs:346` | S3.3 |
| `src/launch.rs:347` | S3.3 |
| `src/main.rs:9` | S3.3 |
| `src/main.rs:29` | S3.3 |
| `src/main.rs:33` | S3.3 |
| `src/main.rs:34` | S3.3 |
| `src/main.rs:38` | S3.3 |
| `src/main.rs:39` | S3.3 |
| `src/main.rs:41` | S3.3 |
| `src/main.rs:42` | S3.3 |
| `src/main.rs:43` | S3.3 |
| `src/main.rs:44` | S3.3 |
| `src/main.rs:46` | S3.3 |
| `src/main.rs:47` | S3.3 |
| `src/main.rs:48` | S3.3 |
| `src/main.rs:51` | S3.3 |
| `src/main.rs:58` | S3.3 |
| `src/main.rs:70` | S3.3 |
| `src/main.rs:71` | S3.3 |
| `src/main.rs:72` | S3.3 |
| `src/main.rs:73` | S3.3 |
| `src/main.rs:74` | S3.3 |
| `src/main.rs:75` | S3.3 |
| `src/desktop_shutdown.rs:221` | S3.5 |
| `src/desktop_shutdown.rs:222` | S3.5 |
| `src/desktop_shutdown.rs:223` | S3.5 |
| `src/desktop_shutdown.rs:256` | S3.10 |
| `src/desktop_shutdown.rs:257` | S3.10 |
| `src/desktop_shutdown.rs:258` | S3.10 |
| `src/desktop_shutdown.rs:263` | S3.9 |
| `src/desktop_shutdown.rs:267` | S3.5 |
| `src/desktop_shutdown.rs:268` | S3.5 |
| `src/desktop_shutdown.rs:269` | S3.5 |
| `src/desktop_shutdown.rs:270` | S3.5 |
| `src/desktop_shutdown.rs:271` | S3.5 |
| `src/desktop_shutdown.rs:272` | S3.5 |
| `src/desktop_shutdown.rs:273` | S3.5 |
| `src/desktop_shutdown.rs:274` | S3.5 |
| `src/desktop_shutdown.rs:460` | S3.9 |
| `src/desktop_shutdown.rs:499` | S3.8 |
| `src/desktop_shutdown.rs:500` | S3.8 |
| `src/desktop_shutdown.rs:501` | S3.8 |
| `src/future_login.rs:354` | S3.7 |
| `src/future_login.rs:355` | S3.7 |
| `src/future_login.rs:482` | S3.7 |
| `src/desktop_shutdown/coordinator.rs:50` | S3.1 |
| `src/desktop_shutdown/coordinator.rs:165` | S3.11 |
| `src/desktop_shutdown/coordinator.rs:166` | S3.11 |
| `src/desktop_shutdown/coordinator.rs:183` | S3.11 |
| `src/desktop_shutdown/coordinator.rs:184` | S3.11 |
| `src/desktop_shutdown/coordinator.rs:185` | S3.11 |
| `src/desktop_shutdown/coordinator.rs:186` | S3.11 |
| `src/desktop_shutdown/coordinator.rs:187` | S3.11 |
| `src/macos_power.rs:21` | S3.2 |
| `src/macos_power.rs:22` | S3.2 |
| `src/macos_power.rs:23` | S3.2 |
| `src/macos_power.rs:24` | S3.2 |
| `src/macos_power.rs:26` | S3.2 |
| `src/macos_power.rs:27` | S3.2 |
| `src/macos_power.rs:29` | S3.2 |
| `src/macos_power.rs:30` | S3.2 |
| `src/macos_power.rs:31` | S3.2 |
| `src/macos_power.rs:32` | S3.2 |
| `src/runtime.rs:248` | S3.8 |
| `src/runtime.rs:249` | S3.8 |
| `src/runtime.rs:251` | S3.8 |
| `src/runtime.rs:252` | S3.8 |
| `src/instance.rs:164` | S3.6 |
| `src/instance.rs:165` | S3.6 |
| `src/instance.rs:166` | S3.6 |
| `src/instance.rs:167` | S3.6 |
| `src/instance.rs:168` | S3.6 |
| `src/bin/futureos-headless.rs:11` | S3.4 |

## S3.5 The gates (PLAN §5.1) and the suite

Run in `desktop/src-tauri`, same environment as §S3.1, **before** the measurement (the
hoists move line numbers and would have invalidated a number taken earlier):

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **0** | clean **crate-wide** — no diff at all, so nothing to attribute to a peer this time. `src/lib.rs` is formatted per file (`rustfmt --edition 2021 --config skip_children=true`), never with a crate-wide `cargo fmt` (PLAN §2b) |
| clippy (gui) | `cargo clippy --all-targets -- -D warnings` | **0** | clean |
| clippy (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **0** | clean |
| this unit's new tests | `cargo test --lib -- --test-threads=1 process_state_initializes_without_a_window the_ready_event_starts_the_window_work_and_serves_the_terminal the_exit_event_closes_admission_and_publishes_the_stop` | **0** | 3 passed / 0 failed / 0 ignored |
| lib suite, gui (measurement) | §S3.1 gui leg | **101** | 2028 passed / **1 failed** / 12 ignored |
| lib suite, headless (measurement) | §S3.1 headless leg | **101** | 1617 passed / **7 failed** / 12 ignored |
| lib suite, gui (first artefact) | `/tmp/tau-c3b-measure.log` gui leg | **0** | 2027 passed / 0 failed / 12 ignored |
| lib suite, headless (first artefact) | `/tmp/tau-c3b-measure.log` headless leg | **101** | 1591 passed / 26 failed / 12 ignored |

**The failures, all outside this unit's scope:**

* gui leg, 1 failure: `remote_peer::runtime_tests::a_supervisor_reports_a_link_that_came_back`
  — `remote: web client bind failed [LC002]: web server bind on port 8022 failed: Address
  already in use (os error 48)` → `the test web port never freed`
  (`src/remote_peer/testing.rs:30`). This is the host failure `.future/cov100/FLAKES.md`
  inventories; the same test **passed** in the first artefact's gui leg (2027/0), i.e. it
  is host/port state, not a regression. `src/remote_peer/**` is not this unit's scope.
* headless leg, 7 failures: six are the same port-8022 family
  (`remote_peer::runtime_tests::{a_bad_link_is_rejected_with_a_shared_support_code,
  a_broken_pairing_does_not_disturb_a_working_one,
  a_burst_of_events_reaches_the_emitter_in_order}`,
  `remote_peer::runtime::tests::attaching_a_stream_to_a_closed_socket_reports_no_stream`,
  `remote_peer::tests::two_hosts_are_kept_apart_in_the_credential_book`,
  `remote::tests::runtime_tests::start_runs_the_full_bridge_and_stop_winds_it_down`) and
  the seventh is `agent_bridge::import::tests::import_missing_sessions_survives_a_panicking_import`
  (`FutureOS: session import parse failed: typed payload is missing or invalid`,
  `src/agent_bridge/import.rs:1220`) — a peer-owned module whose fixture depends on the
  same agent channel the port failure breaks.
* **The headless leg cannot contain this segment's code**: `Cargo.toml` has
  `default = ["gui"]`, `mod gui` is `#[cfg(feature = "gui")]`, and both seams and all
  three tests live inside it — so no line of this diff is compiled into the headless
  binary that reported those 7 failures (or the first artefact's 26).
* The 26-vs-7 spread across the two headless runs of the same revision is the *same*
  load/timing class: in the first artefact the cascade started in `cli_tool::tests`, whose
  fixtures replace `PATH` (`src/cli_tool.rs:515`) and restore it *after* the assertion
  (`:521`) — the fixture panicked at `:518` (`CLI discovery timed out`), so `PATH` stayed
  pointed at an empty temp dir and every later test that shells out to `git` failed with
  `Os { code: 2, NotFound }` (`src/git_review.rs:612-618`, 18 tests), plus
  `tasks::tests::the_headless_loop_runs_a_queued_request_it_is_woken_for`. Reported with
  the mechanism, not chased: both files are peer-owned, and the gui leg of the same
  revision ran those same tests green.

**PLAN §5.2 tripwire** (per-line, mechanical):

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-seg3b.info \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 3" --label tauri-shell-seg3
```

## S3.6 Findings

### S3.6.1 A fixture I wrote was order-dependent, and the suite caught it

The first `Ready` fixture asserted that the Agent saw a `stream_events` request, i.e. that
the observers `start_backend_background` starts had attached. In a targeted run of the
modules that share this process's global state (`desktop_shutdown`, `runtime::`,
`commands::terminal`, `host_environment`, plus this unit's tests) it **failed** —
`the Ready event never started the window's background work` — while passing in isolation.
Mechanism, read from the source: the global observers are once-per-process
(`session_events.rs:20` `static STARTED: AtomicBool` and siblings), and
`mock_agent()` cancels the *per-session* observers but cannot re-arm those flags — so in
a process where a sibling fixture (`backend_background_work_starts_for_any_runtime`) had
already started them, this test had no observable at all. The assertion now targets the
launch path's own post-readiness reconciliation, which is created fresh by
`start_backend_background`, and it is attributable by counting: it requires a **new**
`list_session_ids` command after the dispatch, so a leftover reconciliation cannot
satisfy it. Measured: green in isolation (3 passed) and green in the combined run
(54 passed / 0 failed).

### S3.6.2 Three `DA:0` lines this segment's own tests introduced (taxonomy (e)), hoisted

The first artefact showed `lib.rs` 2114, 2132 and 2140 as `DA:0` — the format arguments
of three multi-line `assert!`s in the new startup test. `assert!`'s format arguments are
evaluated only on failure, so a passing run leaves them unexecuted: the same trap §6.6
recorded and §S2.5 fixed for two lines. Hoisted into `let` bindings with the same message
text and the same failure behaviour; §S3.1 records why that invalidated the first
artefact, and the re-measurement shows all three covered (§S3.6.2 arithmetic).

### S3.6.3 The `_ => {}` arm needs an event nobody handles

`tauri::RunEvent` is `#[non_exhaustive]`, so the wildcard arm is required — and it is
`DA:0` unless a test dispatches a variant this build has no arm for. The `Exit` test now
dispatches `RunEvent::MainEventsCleared` first and asserts the two things the wildcard
must *not* do (close admission, publish a stop) before dispatching `Exit` and asserting
both happen. Measured: 1288 = 1, and the pair of assertions is a contrast, not a call.

### S3.6.4 One line that cannot be observed in the same process as its success direction

`lib.rs:1261` (the `serve`-failure `eprintln!`) is registered rather than covered. The
reason is not "no fixture exists" but "the two directions are mutually exclusive in one
process": `terminal::server::serve()` takes the process-global listener out of its
`Mutex` (`terminal/server.rs:126-135`), so observing the production direction — the arm
serving the listener, which this segment's fixture does assert — consumes the only state
that could make a later `serve()` fail. A child-process fixture can reach it (the crate
already spawns one for the pre-bind arm of the same server,
`commands::terminal::tests::terminal_server_info_reports_not_ready_before_bind`), and
that is the fixture to write; it was not landed here because a test added after the
measurement would invalidate the number this record is written against (§S3.1).

### S3.6.5 The seam block's attribution, measured in the other direction

`lib.rs` 906–924 (the `tauri_plugin_single_instance` callback body: window activation and
the forwarded re-pair request) went from **7** `DA:0` lines in the Segment 2 report
(907, 908, 909, 916, 919, 920, 921) to **12** here (906–910, 916, 917, 919–921, 923, 924)
with no source or test change. The block cannot be executed by any fixture in this suite —
it runs when a *second instance* launches — so both readings are llvm-cov's region
attribution inside a never-executed closure, and §S2.4 already recorded the same block
measured as "30 covered + 6 `DA:0`" and then "36 covered". Consequence for the
arithmetic: the seam's `−17` for `lib.rs` is a *net* figure that includes `+5` of this
drift in untouched code; the structural gain is `−22` against `+5` of attribution. A
reviewer comparing `seg3b` with `seg2b` line-number-by-line-number would otherwise read
these 12 rows as this segment's regression.

### S3.6.6 Two lines in an untouched file flipped the other way

`desktop_shutdown.rs` 499–500, `DA:0` here and covered in Segment 2 — the §S2.8.4 flip,
re-observed. The probe in §S2.8.4 still reproduces (covered in isolation); the mechanism
(a sibling fixture's non-abortable `perform_stop` thread calling `runtime::freeze_tasks()`
between this fixture's `cancel_test_tasks()` and its `spawn`) is unchanged, and the test
passes either way. Recorded because a reviewer who sees the file's count go 17 → 19 must
not read it as this segment's work.

## S3.7 Reproduction

```bash
 # 1. the measurement (§S3.1): bash /tmp/tau-c3b-measure2.sh   (~25 min, both flavours)
 #    2. this unit's new tests, uninstrumented and fast:
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --lib -- --test-threads=1 process_state_initializes_without_a_window
cargo test --lib -- --test-threads=1 the_ready_event_starts_the_window_work_and_serves_the_terminal
cargo test --lib -- --test-threads=1 the_exit_event_closes_admission_and_publishes_the_stop
 # 3. the ordering check that caught §S3.6.1 (this unit's tests + the state they share):
cargo test --lib -- --test-threads=1 desktop_shutdown runtime:: commands::terminal \
    host_environment process_state the_ready_event the_exit_event backend_background
 # 4. the gates in §S3.5
```

# Segment 4 — the last eight open rows are covered; 255 → 242 and **zero open**

Status: **measured** — **242** uncovered lines in this unit's scope, down from **255**
(Segment 3), **270** (Segment 2) and **400** (`8449acf6`). Every one of the 242 is waived
with a named mechanism (§S4.4); the two groups Segment 3 left open are now covered and
**nothing is left un-waived in this unit's scope**. This is this unit's achievable floor:
the remaining lines need a GUI, the macOS main thread, a live launcher process, real
power notifications, a `cfg(not(test))` build, a Linux host, or they are braces
llvm-cov attributes to a block end — the same four structural families as before, and the
seams that would shrink them are §5.2/§5.3 (production-shape proposals, not fixtures).

What changed, in one paragraph: the two fixtures Segment 3 named as its open work were
landed — a **child-process fixture** for the `serve`-failure arm of the `Ready` path
(`lib.rs:1261`) and two **store-state fixtures** for the post-readiness reconciliation
(`lib.rs` 1347 and 1355–1360, driven by an unopenable store and by a planted empty
conversation) — and the fd capture the third one needs was generalized from fd 1 to any
descriptor (`FdCapture`). Eight target lines flipped zero → covered, verified per line;
**no** line of this unit regressed, and no test body introduced a `DA:0` line this time
(§S4.3). Two of the eight are reached only from a *separate process*, which also answers
the campaign's open question about spawned instrumented children (§S4.7.2). The
measurement method needed a fix of its own (§S4.1/§S4.7.3): the single-report recipe lost
a whole flavour's late-run data on this host in this run, so the record is built from
**one lcov per flavour, unioned per line**.

## S4.1 Run identity and the exact measurement

macOS arm64 (Darwin 25.x), Rust 1.97.0, `--test-threads=1` (mandatory on this host),
revision: this unit's `src/lib.rs` after the Segment-4 test additions — the file that is
in commit `774baf0e` (verified byte-identical to the working tree when the measurement was
taken: `git show 774baf0e:desktop/src-tauri/src/lib.rs | shasum` =
`shasum desktop/src-tauri/src/lib.rs` = `ce3ba81f547f`). The supervisor's snapshots carry
the file between revisions, so this is the commit to check out to reproduce the numbers.

**Two per-flavour legs, each rendered while its own build is current, then unioned per
line.** The `gui` leg:

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-shell
ulimit -n 10240
cargo llvm-cov clean --manifest-path Cargo.toml                                 # exit 0
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
 #   → exit 101: lib 2052 passed / 1 failed / 12 ignored
cargo llvm-cov report --manifest-path Cargo.toml --lcov \
    --output-path /tmp/tauri-shell-lcov-gui-only.info                            # exit 0
```

then the same three commands with `--no-default-features --features headless`
(`/tmp/tau-c4-headless-only.sh`, log `/tmp/tau-c4-headless-only.log`,
15:56:40→16:05:30 UTC):

```bash
cargo llvm-cov clean --manifest-path Cargo.toml                                 # exit 0
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml \
    --no-default-features --features headless -- --test-threads=1
 #   → exit 0: lib 1636 passed / 0 failed / 12 ignored   (green)
cargo llvm-cov report --manifest-path Cargo.toml --lcov \
    --output-path /tmp/tauri-shell-lcov-headless-only.info                       # exit 0
cd /Users/geilige/future-os/.worktrees/cov100-r2
python3 /tmp/lcov-union.py --out /tmp/tauri-shell-lcov-seg4union.info \
    /tmp/tauri-shell-lcov-gui-only.info /tmp/tauri-shell-lcov-headless-only.info # 170 files
```

The union marks a line covered iff either flavour covered it — for the `DA:0` metric that
is what the campaign's merged report means, and it is the report this section's numbers
come from. Work list:

```bash
python3 .future/cov100/covlines.py total /tmp/tauri-shell-lcov-seg4union.info \
    --under desktop/src-tauri/src/lib.rs        # …one --under per file in the scope
python3 .future/cov100/covlines.py detail /tmp/tauri-shell-lcov-seg4union.info \
    --under desktop/src-tauri/src/lib.rs
```

**Why not the single-report recipe.** The campaign command (gui leg → headless leg → one
`report`) was run first (`/tmp/tau-c4-measure.sh`, log `/tmp/tau-c4-measure.log`,
15:21→15:35 UTC, artifact `/tmp/tauri-shell-lcov-seg4.info`): `PLACEHOLDER_EXIT=0`,
`CLEAN_EXIT=0`, `GUI_RUN_EXIT=0`, `HEADLESS_RUN_EXIT=101` (1635 passed / 1 failed /
12 ignored), `REPORT_EXIT=0`. Its report claims **348** uncovered lines — a number that is
**wrong**, and the record has to say so rather than publish it. Three independent signals
agree that the gui leg's late-run data never reached that report:

1. its gui leg is **green** (`exit 0`, no failures) while the same revision's gui leg run
   in isolation has one failing test (`skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`),
   i.e. the combined gui leg never ran that late test;
2. its log stops printing per-test lines at the `gui::…` tests and loses the
   `macos_power::tests::…` line entirely (the isolated run has it);
3. its report shows `macos_power.rs` **40/40** uncovered where both the isolated gui leg
   and Segment 3 show **10/40**, and `runtime.rs:42` (a gui-only test helper) covered → 0.

Re-rendering that report from the same profraw produced a byte-identical file (`cmp` exit
0), so the render is deterministic and the data — not the tool — is what went missing.
Rendering per flavour right after its own leg gives the correct picture for both
(`macos_power.rs` 10/40, `runtime.rs` 4/216 — exactly Segment 3's), so the union above is
the artifact of record. §S4.7.3 states the finding for the campaign: the single-report
step resolves profiles against the build that is *current* when it runs, so a
previous-flavour leg can be dropped; that failure mode is silent (exit 0).

## S4.2 Before and after, per file

Uncovered lines (`DA:0`), each column re-derived from its own report with
`covlines.py total … --under <file>`:

| file | `8449acf6` | Segment 2 | Segment 3 | **this segment** |
|---|---|---|---|---|
| `src/lib.rs` | 203 | 138 | 121 | **108** |
| `src/menu.rs` | 48 | 48 | 48 | 48 |
| `src/launch.rs` | 38 | 15 | 15 | 15 |
| `src/main.rs` | 28 | 21 | 21 | 21 |
| `src/desktop_shutdown.rs` | 20 | 17 | 19 | 19 |
| `src/future_login.rs` | 18 | 3 | 3 | 3 |
| `src/desktop_shutdown/coordinator.rs` | 15 | 8 | 8 | 8 |
| `src/macos_power.rs` | 10 | 10 | 10 | 10 |
| `src/runtime.rs` | 8 | 4 | 4 | 4 |
| `src/instance.rs` | 7 | 5 | 5 | 5 |
| `src/host_environment.rs` | 0 | 0 | 0 | 0 |
| `src/bin/futureos-headless.rs` | 5 | 1 | 1 | 1 |
| **total** | **400** | **270** | **255** | **242** |

Measured (non-zero) lines in the same scope: **3687 → 3839** (`lib.rs` 971 → 1123; the
+152 is this segment's test code and the `FdCapture` generalization, all of it inside
`#[cfg(test)]`).

`lib.rs` **121 → 108 = −13**, and every one of the 13 is a zero → non-zero flip on code
whose *text* is identical between the two revisions (compared per line, not by number):

* **8 are this segment's**: `1261` (1 hit), `1347` (1), `1355`–`1360` (1 each) — the two
  fixtures below;
* **5 are the attribution drift Segment 2 documented and this segment does not claim**:
  `906`, `910`, `917`, `923`, `924` — inside the `tauri_plugin_single_instance` callback
  body that only a second-instance launch can execute. Segment 3 measured that block as
  12 `DA:0` lines, this run has 7 (`907–909`, `916`, `919–921`), Segment 2 measured it as
  "30 covered + 6 `DA:0`" and then as 36 covered. Nothing in this segment touches that
  block or any test that reaches it.

**Zero flips in the other direction**: no production line moved from covered to `DA:0`,
and **no line in the test region is `DA:0`** (0 zero→non-zero and 0 non-zero→zero flips in
`lib.rs` ≥ 1900), i.e. the taxonomy-(e) trap that cost Segment 3 an extra measurement did
not recur — the new assertions hoist their format arguments (§S4.3).

## S4.3 The two fixtures (and the test-side refactor they needed)

**(1) `lib.rs:1261` — the `serve`-failure arm, from a child process.**
`a_ready_event_whose_listener_is_already_taken_reports_the_serve_failure` takes the
terminal listener in a **fresh process** (`bind()` then `serve()`), dispatches
`RunEvent::Ready`, and asserts on the child's stderr that the arm reported
`FutureOS terminal server failed to start`. This is the fixture §S3.12 named, in the shape
this crate already uses (`commands::terminal::tests` spawns `current_exe` for the pre-bind
arm of the same server). The child gets its own `HOME`, so the launch path it drives
cannot resolve this process's store. **Measured: `1261` = 1.**

**(2) `lib.rs` 1347 and 1355–1360 — the post-readiness reconciliation's three arms.**
Two fixtures, because the arms are mutually exclusive inputs:

* `a_broken_store_makes_the_startup_reconciliation_report_its_failures`: `HomeGuard` +
  mock agent + `initialize_app_store`, then the **database path is replaced by a
  directory** so `Connection::open` fails; both `Err` arms run (`1347`, `1359–1360`) and
  the removal message must **not** appear — asserted, so the negative direction is
  evidence and not silence. A directory, deliberately not "garbage bytes": a zero-filled
  header is what SQLite treats as a *new* database, and that shape would have made the
  fixture assert nothing.
* `an_empty_conversation_is_removed_by_the_startup_reconciliation`: plants one thread row
  with no run (`agent_session_id` set, `created_at = 1`, past the 10-minute grace), tells
  the mock Agent that the session **is** live (otherwise the first statement of the block
  hard-deletes it as an orphan) and that its journal page is empty, then asserts both the
  count in the report (`removed 1 empty conversation(s)`) and that the row is gone in the
  database afterwards. **Measured: `1347` = 1, `1355` = 1, `1356` = 1, `1357` = 1,
  `1358` = 1, `1359` = 1, `1360` = 1.**

**The refactor they needed:** the child fixtures must read a *process-level* stderr, and
the two store fixtures need the same capability. `StdoutCapture` (Segment 2, fd 1) became
`FdCapture` with `stdout()`/`stderr()` constructors, a `text()` reader and a
label-carrying timeout message. Test-only, no production line touched; the existing
`the_terminal_delivery_prints_the_launcher_notice_to_this_stdout` uses it unchanged.

**No production code changed in this segment.** The eight lines became reachable by
*input*, not by a seam — which is why §4b needed no new approval here.

## S4.4 The residual (242 lines) — categories

| group | lines | category | what places the line there |
|---|---|---|---|
| S4.1 the real Tauri/Wry application — `run_with`'s own body (888–890, 1195–1206), the plugin/menu/window hook bodies (907–924, 932–975), the window-geometry helpers (209–215, 474–478), the runtime-update emitters (811, 842, 1004–1040), `coordinator.rs:50` | 98 | `unreachable-in-this-environment` | `Builder::<Wry>::build` creates the Wry event loop and Tauri's default macOS menu, and `muda` only builds menus on the main thread (§6.2: a libtest body is never the main thread). The hook *bodies* additionally need real window/menu/IPC events: 907–924 is the second-instance callback, 932–975 the menu and window-event handlers. The 907–924 part is attribution-unstable between runs (§S4.2, §S3.6.5) |
| S4.2 macOS main-thread-only — `menu.rs` (48) + `macos_power.rs` (10) | 58 | `unreachable-in-this-environment` | `build_macos_menu` constructs `muda` items and asserts `NSThread::isMainThread`; the power hooks are AppKit/IOKit notifications. Unchanged from §4.2/§4.3 |
| S4.3 the launcher *process* bodies — `launch.rs` 330–347, `main.rs`'s remaining arms | 36 | `unreachable-in-this-environment` | these bodies spawn a real GUI Desktop and wait for its answer; §4.4's waiver, unchanged |
| S4.4 the Linux-only helper success arm | 1 | `platform-unmeasured` | `bin/futureos-headless.rs:11` compiles only on Linux |
| S4.5 `cfg(not(test))`-excluded blocks — `desktop_shutdown.rs` 221–223, 267–274 | 11 | `unreachable-by-construction` | both sit behind `#[cfg(all(feature = "gui", not(test)))]`; every test build removes them. Invariant: the test cfg |
| S4.6 the `flock` re-acquire window | 5 | `unreachable-in-this-environment` | `instance.rs` 164–168 needs a lock taken *inside* the re-acquire window. Unchanged from §4.7 |
| S4.7 client-stack arms | 3 | `unreachable-in-this-environment` | `future_login.rs` 354, 355, 482: §6.5's measured result — this host's stack reports body-transfer failures as decode errors, so the `is_body()`/`is_connect()` arms are not reachable with any of the three fixtures tried |
| S4.8 never-evaluated and fixture-only arms | 10 | `unreachable-by-construction` | `lib.rs:1727` (the burst producer's early `return`: it needs the drain loop to have returned, which the fixture forbids before the burst ends), 1906–1907 (`Failed::flush`'s `Ok`: the two writer cases are "write fails" and "flush fails"); `runtime.rs` 248, 249, 251, 252 (the frozen registry's two closure bodies — the test asserts they never run); `desktop_shutdown.rs:501` (the `std::future::pending` sentinel). Every one sits in **test** code, where the fixture's own design *is* the invariant |
| S4.9 region llvm-cov attributes to a block-end brace | 3 | `attribution-artifact` | `lib.rs:1398` closes `if !enabled { return; }` (1396–1397 covered); `desktop_shutdown.rs` 263 and 460 close blocks whose bodies are covered (260–262, 457–459) |
| S4.10 downstream of the remote bridge's private state | 10 | `unreachable-in-this-environment` | `lib.rs` 1400–1411 (the spawned auto-connect pass) and `desktop_shutdown.rs` 256–258 (the remote-shutdown deadline arm): reaching them needs `remote::SUPERVISOR.state` to be `Some`, whose only installer (`remote::tests::runtime_tests::install_state`/`fake_state`) is private to `src/remote/**` — outside this unit's write set. The gate they sit behind is covered; the pass is not (§S2.8.3, handoff `todo_2824cf77774d`) |
| S4.11 an OS thread-creation refusal | 7 | `unreachable-in-this-environment` | `coordinator.rs` 165–166 and 183–187 are the two `std::thread::Builder::spawn` error arms: forcing one needs an OS refusal the suite cannot induce without changing a process-wide limit; the thread names are compile-time constants, so there is no injection point |
| **total** | **242** | | |

**Arithmetic:** 217 `unreachable-in-this-environment` (S4.1 98 + S4.2 58 + S4.3 36 + S4.6 5
+ S4.7 3 + S4.10 10 + S4.11 7) + 21 `unreachable-by-construction` (S4.5 11 + S4.8 10) +
1 `platform-unmeasured` (S4.4) + 3 `attribution-artifact` (S4.9) = **242 waived, 0 open**.
`255 − 13 = 242`, of which 8 lines are this segment's and 5 are the drift quoted in §S4.2.
The per-line index below has exactly 242 rows, one per residual line.

Checked mechanically before hand-off (PLAN §5.2) — §S4.6 has the invocation and its exit
code.

### Per-line index of this segment's residual (242 rows)

One row per residual line of this revision, generated from
`/tmp/tauri-shell-lcov-seg4union.info` with the groups above. §8, §S2.6 and §S3.4 keep
their own revisions' rows.

| line | group |
|---|---|
| `src/lib.rs:209` | S4.1 |
| `src/lib.rs:210` | S4.1 |
| `src/lib.rs:211` | S4.1 |
| `src/lib.rs:212` | S4.1 |
| `src/lib.rs:213` | S4.1 |
| `src/lib.rs:214` | S4.1 |
| `src/lib.rs:215` | S4.1 |
| `src/lib.rs:474` | S4.1 |
| `src/lib.rs:475` | S4.1 |
| `src/lib.rs:476` | S4.1 |
| `src/lib.rs:477` | S4.1 |
| `src/lib.rs:478` | S4.1 |
| `src/lib.rs:811` | S4.1 |
| `src/lib.rs:842` | S4.1 |
| `src/lib.rs:888` | S4.1 |
| `src/lib.rs:889` | S4.1 |
| `src/lib.rs:890` | S4.1 |
| `src/lib.rs:907` | S4.1 |
| `src/lib.rs:908` | S4.1 |
| `src/lib.rs:909` | S4.1 |
| `src/lib.rs:916` | S4.1 |
| `src/lib.rs:919` | S4.1 |
| `src/lib.rs:920` | S4.1 |
| `src/lib.rs:921` | S4.1 |
| `src/lib.rs:932` | S4.1 |
| `src/lib.rs:933` | S4.1 |
| `src/lib.rs:934` | S4.1 |
| `src/lib.rs:935` | S4.1 |
| `src/lib.rs:936` | S4.1 |
| `src/lib.rs:937` | S4.1 |
| `src/lib.rs:941` | S4.1 |
| `src/lib.rs:942` | S4.1 |
| `src/lib.rs:943` | S4.1 |
| `src/lib.rs:945` | S4.1 |
| `src/lib.rs:950` | S4.1 |
| `src/lib.rs:955` | S4.1 |
| `src/lib.rs:956` | S4.1 |
| `src/lib.rs:957` | S4.1 |
| `src/lib.rs:958` | S4.1 |
| `src/lib.rs:960` | S4.1 |
| `src/lib.rs:962` | S4.1 |
| `src/lib.rs:963` | S4.1 |
| `src/lib.rs:964` | S4.1 |
| `src/lib.rs:965` | S4.1 |
| `src/lib.rs:967` | S4.1 |
| `src/lib.rs:968` | S4.1 |
| `src/lib.rs:969` | S4.1 |
| `src/lib.rs:970` | S4.1 |
| `src/lib.rs:971` | S4.1 |
| `src/lib.rs:974` | S4.1 |
| `src/lib.rs:975` | S4.1 |
| `src/lib.rs:980` | S4.1 |
| `src/lib.rs:984` | S4.1 |
| `src/lib.rs:985` | S4.1 |
| `src/lib.rs:986` | S4.1 |
| `src/lib.rs:991` | S4.1 |
| `src/lib.rs:992` | S4.1 |
| `src/lib.rs:993` | S4.1 |
| `src/lib.rs:995` | S4.1 |
| `src/lib.rs:1004` | S4.1 |
| `src/lib.rs:1005` | S4.1 |
| `src/lib.rs:1006` | S4.1 |
| `src/lib.rs:1007` | S4.1 |
| `src/lib.rs:1008` | S4.1 |
| `src/lib.rs:1010` | S4.1 |
| `src/lib.rs:1011` | S4.1 |
| `src/lib.rs:1031` | S4.1 |
| `src/lib.rs:1032` | S4.1 |
| `src/lib.rs:1033` | S4.1 |
| `src/lib.rs:1034` | S4.1 |
| `src/lib.rs:1035` | S4.1 |
| `src/lib.rs:1036` | S4.1 |
| `src/lib.rs:1037` | S4.1 |
| `src/lib.rs:1038` | S4.1 |
| `src/lib.rs:1039` | S4.1 |
| `src/lib.rs:1040` | S4.1 |
| `src/lib.rs:1195` | S4.1 |
| `src/lib.rs:1196` | S4.1 |
| `src/lib.rs:1198` | S4.1 |
| `src/lib.rs:1204` | S4.1 |
| `src/lib.rs:1205` | S4.1 |
| `src/lib.rs:1206` | S4.1 |
| `src/lib.rs:1227` | S4.1 |
| `src/lib.rs:1252` | S4.1 |
| `src/lib.rs:1253` | S4.1 |
| `src/lib.rs:1256` | S4.1 |
| `src/lib.rs:1267` | S4.1 |
| `src/lib.rs:1268` | S4.1 |
| `src/lib.rs:1270` | S4.1 |
| `src/lib.rs:1271` | S4.1 |
| `src/lib.rs:1272` | S4.1 |
| `src/lib.rs:1273` | S4.1 |
| `src/lib.rs:1275` | S4.1 |
| `src/lib.rs:1276` | S4.1 |
| `src/lib.rs:1277` | S4.1 |
| `src/lib.rs:1278` | S4.1 |
| `src/lib.rs:1279` | S4.1 |
| `src/lib.rs:1398` | S4.9 |
| `src/lib.rs:1400` | S4.10 |
| `src/lib.rs:1401` | S4.10 |
| `src/lib.rs:1402` | S4.10 |
| `src/lib.rs:1403` | S4.10 |
| `src/lib.rs:1405` | S4.10 |
| `src/lib.rs:1409` | S4.10 |
| `src/lib.rs:1411` | S4.10 |
| `src/lib.rs:1727` | S4.8 |
| `src/lib.rs:1906` | S4.8 |
| `src/lib.rs:1907` | S4.8 |
| `src/menu.rs:26` | S4.2 |
| `src/menu.rs:27` | S4.2 |
| `src/menu.rs:28` | S4.2 |
| `src/menu.rs:29` | S4.2 |
| `src/menu.rs:31` | S4.2 |
| `src/menu.rs:32` | S4.2 |
| `src/menu.rs:33` | S4.2 |
| `src/menu.rs:34` | S4.2 |
| `src/menu.rs:35` | S4.2 |
| `src/menu.rs:36` | S4.2 |
| `src/menu.rs:37` | S4.2 |
| `src/menu.rs:38` | S4.2 |
| `src/menu.rs:39` | S4.2 |
| `src/menu.rs:40` | S4.2 |
| `src/menu.rs:41` | S4.2 |
| `src/menu.rs:42` | S4.2 |
| `src/menu.rs:43` | S4.2 |
| `src/menu.rs:44` | S4.2 |
| `src/menu.rs:45` | S4.2 |
| `src/menu.rs:46` | S4.2 |
| `src/menu.rs:47` | S4.2 |
| `src/menu.rs:48` | S4.2 |
| `src/menu.rs:49` | S4.2 |
| `src/menu.rs:51` | S4.2 |
| `src/menu.rs:52` | S4.2 |
| `src/menu.rs:53` | S4.2 |
| `src/menu.rs:54` | S4.2 |
| `src/menu.rs:55` | S4.2 |
| `src/menu.rs:56` | S4.2 |
| `src/menu.rs:57` | S4.2 |
| `src/menu.rs:58` | S4.2 |
| `src/menu.rs:59` | S4.2 |
| `src/menu.rs:61` | S4.2 |
| `src/menu.rs:62` | S4.2 |
| `src/menu.rs:63` | S4.2 |
| `src/menu.rs:65` | S4.2 |
| `src/menu.rs:66` | S4.2 |
| `src/menu.rs:67` | S4.2 |
| `src/menu.rs:68` | S4.2 |
| `src/menu.rs:69` | S4.2 |
| `src/menu.rs:70` | S4.2 |
| `src/menu.rs:72` | S4.2 |
| `src/menu.rs:73` | S4.2 |
| `src/menu.rs:74` | S4.2 |
| `src/menu.rs:75` | S4.2 |
| `src/menu.rs:76` | S4.2 |
| `src/menu.rs:77` | S4.2 |
| `src/menu.rs:78` | S4.2 |
| `src/launch.rs:330` | S4.3 |
| `src/launch.rs:331` | S4.3 |
| `src/launch.rs:332` | S4.3 |
| `src/launch.rs:333` | S4.3 |
| `src/launch.rs:334` | S4.3 |
| `src/launch.rs:338` | S4.3 |
| `src/launch.rs:339` | S4.3 |
| `src/launch.rs:340` | S4.3 |
| `src/launch.rs:341` | S4.3 |
| `src/launch.rs:342` | S4.3 |
| `src/launch.rs:343` | S4.3 |
| `src/launch.rs:344` | S4.3 |
| `src/launch.rs:345` | S4.3 |
| `src/launch.rs:346` | S4.3 |
| `src/launch.rs:347` | S4.3 |
| `src/main.rs:9` | S4.3 |
| `src/main.rs:29` | S4.3 |
| `src/main.rs:33` | S4.3 |
| `src/main.rs:34` | S4.3 |
| `src/main.rs:38` | S4.3 |
| `src/main.rs:39` | S4.3 |
| `src/main.rs:41` | S4.3 |
| `src/main.rs:42` | S4.3 |
| `src/main.rs:43` | S4.3 |
| `src/main.rs:44` | S4.3 |
| `src/main.rs:46` | S4.3 |
| `src/main.rs:47` | S4.3 |
| `src/main.rs:48` | S4.3 |
| `src/main.rs:51` | S4.3 |
| `src/main.rs:58` | S4.3 |
| `src/main.rs:70` | S4.3 |
| `src/main.rs:71` | S4.3 |
| `src/main.rs:72` | S4.3 |
| `src/main.rs:73` | S4.3 |
| `src/main.rs:74` | S4.3 |
| `src/main.rs:75` | S4.3 |
| `src/desktop_shutdown.rs:221` | S4.5 |
| `src/desktop_shutdown.rs:222` | S4.5 |
| `src/desktop_shutdown.rs:223` | S4.5 |
| `src/desktop_shutdown.rs:256` | S4.10 |
| `src/desktop_shutdown.rs:257` | S4.10 |
| `src/desktop_shutdown.rs:258` | S4.10 |
| `src/desktop_shutdown.rs:263` | S4.9 |
| `src/desktop_shutdown.rs:267` | S4.5 |
| `src/desktop_shutdown.rs:268` | S4.5 |
| `src/desktop_shutdown.rs:269` | S4.5 |
| `src/desktop_shutdown.rs:270` | S4.5 |
| `src/desktop_shutdown.rs:271` | S4.5 |
| `src/desktop_shutdown.rs:272` | S4.5 |
| `src/desktop_shutdown.rs:273` | S4.5 |
| `src/desktop_shutdown.rs:274` | S4.5 |
| `src/desktop_shutdown.rs:460` | S4.9 |
| `src/desktop_shutdown.rs:499` | S4.8 |
| `src/desktop_shutdown.rs:500` | S4.8 |
| `src/desktop_shutdown.rs:501` | S4.8 |
| `src/future_login.rs:354` | S4.7 |
| `src/future_login.rs:355` | S4.7 |
| `src/future_login.rs:482` | S4.7 |
| `src/desktop_shutdown/coordinator.rs:50` | S4.1 |
| `src/desktop_shutdown/coordinator.rs:165` | S4.11 |
| `src/desktop_shutdown/coordinator.rs:166` | S4.11 |
| `src/desktop_shutdown/coordinator.rs:183` | S4.11 |
| `src/desktop_shutdown/coordinator.rs:184` | S4.11 |
| `src/desktop_shutdown/coordinator.rs:185` | S4.11 |
| `src/desktop_shutdown/coordinator.rs:186` | S4.11 |
| `src/desktop_shutdown/coordinator.rs:187` | S4.11 |
| `src/macos_power.rs:21` | S4.2 |
| `src/macos_power.rs:22` | S4.2 |
| `src/macos_power.rs:23` | S4.2 |
| `src/macos_power.rs:24` | S4.2 |
| `src/macos_power.rs:26` | S4.2 |
| `src/macos_power.rs:27` | S4.2 |
| `src/macos_power.rs:29` | S4.2 |
| `src/macos_power.rs:30` | S4.2 |
| `src/macos_power.rs:31` | S4.2 |
| `src/macos_power.rs:32` | S4.2 |
| `src/runtime.rs:248` | S4.8 |
| `src/runtime.rs:249` | S4.8 |
| `src/runtime.rs:251` | S4.8 |
| `src/runtime.rs:252` | S4.8 |
| `src/instance.rs:164` | S4.6 |
| `src/instance.rs:165` | S4.6 |
| `src/instance.rs:166` | S4.6 |
| `src/instance.rs:167` | S4.6 |
| `src/instance.rs:168` | S4.6 |
| `src/bin/futureos-headless.rs:11` | S4.4 |


## S4.5 The gates (PLAN §5.1) and the suites

Run in `desktop/src-tauri`, same environment as §S4.1, on this revision (gates before the
measurement, because the new tests move line numbers):

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **1** | 4 diffs, **all in peer-owned files**: `src/agent_bridge/headless.rs:382,393` and `src/agent_bridge/tests.rs:2136,2156` (a peer unit's in-flight test code; `src/agent_bridge/**` is not in this unit's write set). **This unit's file is clean**: `rustfmt --edition 2021 --config skip_children=true --check src/lib.rs` → **0**, and `desktop_shutdown.rs` was not touched this segment. A per-file `rustfmt` (never a crate-wide `cargo fmt`, PLAN §2b) is what this unit uses |
| clippy (gui) | `cargo clippy --all-targets -- -D warnings` | **0** | clean, after fixing three `clippy::needless_return` errors **this unit introduced** (§S4.7.1) |
| clippy (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **0** | clean |
| lib suite, gui (instrumented leg) | `/tmp/tau-c4-gui-only.sh`, leg 1 | **101** | `2052 passed; 1 failed; 12 ignored` — the failure is `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin` (peer-owned `skills_bootstrap.rs`/`cli_tool.rs`, the discovery-timeout family §S3.5 documented) |
| lib suite, headless (instrumented leg) | `/tmp/tau-c4-headless-only.sh`, leg 1 | **0** | `1636 passed; 0 failed; 12 ignored` — **green** |
| this unit's new fixtures | `cargo test --lib -- --test-threads=1 a_ready_event_whose_listener_is_already_taken… a_broken_store_makes… an_empty_conversation_is_removed…` | **0** | 3 passed / 0 failed |
| this unit's whole fixture set + the state they share | `cargo test --lib -- --test-threads=1 process_state… the_ready_event… the_exit_event… backend_background… the_terminal_delivery… a_host_without_a_resolvable_cli… a_success_after_the_deadline… a_lock_failure_that_is_not_contention… unconfirmed_runs_report… runtime_update_drain_loop… <the three new names>` | **0** | 14 passed / 0 failed — the ordering check that a fixture of this unit must pass |
| the single-report recipe (not the record, §S4.1) | `/tmp/tau-c4-measure.sh` | gui **0**, headless **101**, report **0** | gui `2028 passed / 1 failed / 12 ignored` *as logged*, but that leg is truncated (§S4.1); headless `1635 passed / 1 failed / 12 ignored` (`remote_peer::runtime_tests::connect_all_brings_up_every_pairing_and_names_the_ones_it_cannot`, the port-8022 host failure) |

The one failing gui test and the one failing headless test are the same two classes
Segment 3 reported: a **fixed test port** (`8022`) that other crate-test processes on this
host hold, and the `cli_tool` **discovery timeout** (a `PATH`-replacing fixture under
load). Both live in peer-owned modules; neither is in this unit's write set, and the
headless leg of this revision is green when it runs alone.

## S4.6 PLAN §5.2 tripwire (per-line, mechanical)

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-seg4union.info \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 4" --label tauri-shell-seg4
```

## S4.7 Findings

### S4.7.1 The clippy gate caught three errors in this segment's own tests

`cargo clippy --all-targets -- -D warnings` failed with three
`clippy::needless_return` errors at the end of the new fixtures' parent branches. They are
exactly the class of failure that a green test suite hides (the suite passed with them, and
they would have reddened CI's Rust job). Fixed by deleting the three `return;` statements —
they were the last statement of each test body, so removal is behaviour-identical — and the
gate is green. Recorded because it is the second time in this campaign that the *gate*, not
the suite, was what noticed.

### S4.7.2 A spawned child's profile does reach the merged report — and the harness captures `eprintln!` from every thread

Two facts a test in this crate needs, both measured here:

1. **The child's coverage counted.** `lib.rs:1261` and the seven reconciliation lines are
   executed **only** inside the child processes this segment's fixtures spawn, and all
   eight appear with a non-zero counter in the per-flavour report. The profraw files
   confirm the mechanism: cargo-llvm-cov's pattern carries the pid
   (`src-tauri-<pid>-<hash>_0.profraw`), so parent and child each write their own file and
   the merge sums them. That settles the open question the TAU-C3 analysis left for §5.3
   (a main-thread helper *process* would, in principle, contribute) — the plumbing is not
   the blocker; the *main thread* is (§6.2: a spawned process is not a libtest thread, but
   a spawned process also cannot construct this app's real Wry runtime without a display).
2. **`eprintln!` is captured for every thread of the test process in this toolchain.** The
   first version of these fixtures redirected fd 2 in-process and read nothing, while the
   messages showed up in libtest's own captured output — including messages from
   `std::thread::spawn`ed maintenance jobs, i.e. the harness's capture is not per-thread
   here. `--nocapture` in a child process is therefore the way to observe a message the
   host prints; §S2.8.2's fd-1 capture works for the *opposite* reason (a
   `std::io::Stdout` write below the macro layer).

### S4.7.3 The campaign's single-report measurement step is silently lossy (host-measured)

The gui leg in the single-report run was **truncated**: it exited 0 while its report, its
log and its test outcomes all show that everything after the `gui::…` tests never ran
(§S4.1: a missing late failing test, missing per-test lines, `macos_power.rs` 40/40
instead of 10/40, and a gui-only test helper at `runtime.rs:42` covered → 0). The render
itself is deterministic (`cmp` on two renders: identical), so the data never reached it:
`cargo llvm-cov report` resolves the profraw against the build that is *current* when it
runs, and the headless leg rebuilt the crate after the gui leg. Consequence for this
campaign: a single-report number can be too *high* (it under-reports coverage) with no
error anywhere — the exact failure direction the campaign's "zeroed report" guards do not
catch, because the report is well-formed and non-empty. The fix used here (render per
flavour immediately after that flavour's leg, union per line) is cheap and belongs in
`TAURI.md`; the two-leg legs themselves are unaffected.

### S4.7.4 The removal arm needs three inputs, and two of them are easy to get wrong

To reach `lib.rs:1355–1358` the fixture needed (a) an **unreadable** store — but only for
the removal arm's *sibling*, so the two arms need separate fixtures; (b) a thread row that
the first statement of the block will *not* hard-delete, which means the mock Agent must
report the planted session as live (otherwise the row is an orphan and is gone before the
emptiness probe runs); and (c) an emptiness probe that **succeeds** with "no messages" —
which requires the `get_session_entries` reply to decode as a page (a JSON `data` reply
works through the decoder's fallback; the default mock reply is a generic `{}` and would
land in the probe-*failed* arm instead). Each of the three was a wrong first attempt.

### S4.7.5 Attribution pinned where a message is duplicated

`agent_bridge::observer`'s 60-second discovery backstop prints the **same two strings**
this segment's target lines print (`removed {n} empty conversation(s)`,
`empty-conversation reconcile failed`). The fixture therefore pins that tick away for its
duration with `FUTURE_TEST_DISCOVERY_INTERVAL_MS` (a cfg(test)-only seam read on every
iteration) — in the *child*, so the value cannot leak into any other test. Without it the
assertions could have been satisfied by the other module, which is a false attribution
rather than a false pass.

## S4.8 Reproduction

```bash
 # 1. the measurement (§S4.1): the two per-flavour legs, then the union
bash /tmp/tau-c4-gui-only.sh          # gui leg      (~10 min)
bash /tmp/tau-c4-headless-only.sh     # headless leg (~9 min, green)
python3 /tmp/lcov-union.py --out /tmp/tauri-shell-lcov-seg4union.info \
    /tmp/tauri-shell-lcov-gui-only.info /tmp/tauri-shell-lcov-headless-only.info
 # 2. this unit's new fixtures, uninstrumented and fast
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --lib -- --test-threads=1 a_ready_event_whose_listener_is_already_taken_reports_the_serve_failure
cargo test --lib -- --test-threads=1 a_broken_store_makes_the_startup_reconciliation_report_its_failures
cargo test --lib -- --test-threads=1 an_empty_conversation_is_removed_by_the_startup_reconciliation
 # 3. the ordering check (§S4.5) and the gates
cargo test --lib -- --test-threads=1 desktop_shutdown runtime:: commands::terminal \
    host_environment instance:: process_state the_ready_event the_exit_event backend_background
cargo fmt --check ; cargo clippy --all-targets -- -D warnings
 # 4. the tripwire (§S4.6)
```

# Segment 5 — the Segment-4 measurement reproduced (+3 = the two known instabilities, nothing else), the tripwire re-checked against the new STALE half, and the main-thread proposal measured as far as this scope allows

Status: **same revision as Segment 4 — no source change in this segment** (`git status` for
every path in this unit's scope is clean; the only file written is this record). Pinned
residual **242** (§S4.4 stays the authoritative ledger and its 242-row index is unchanged —
this segment adds no rows and changes no line number). The re-run measured **245**; the
difference is fully attributed in §S5.1 to the two instabilities this record had already
named, and **no third line moved**.

## S5.1 The reproduction (todo item 1)

The two per-flavour legs and the union were re-run on the frozen revision
(`/tmp/tau-c5-repro.sh`, log `/tmp/tau-c5-repro.log`, 16:14:27→16:40:41 UTC), with the
gates already green beforehand:

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH \
       CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup \
       HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home \
       CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-shell
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder   # exit 0
cargo llvm-cov clean --manifest-path Cargo.toml                                     # exit 0
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
 #   → exit 101: lib 2059 passed / 1 failed / 12 ignored      (gui, full run this time)
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tauri-shell-lcov-c5-gui.info
 #   → exit 0
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml \
    --no-default-features --features headless -- --test-threads=1
 #   → exit 101: lib 1636 passed / 6 failed / 12 ignored      (headless)
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tauri-shell-lcov-c5-headless.info
 #   → exit 0
python3 /tmp/lcov-union.py --out /tmp/tauri-shell-lcov-c5union.info \
    /tmp/tauri-shell-lcov-c5-gui.info /tmp/tauri-shell-lcov-c5-headless.info           # exit 0
```

Per file, pinned (Segment 4) versus this run — `covlines.py total … --under <file>`:

| file | pinned | this run | note |
|---|---|---|---|
| `src/lib.rs` | 108 | **113** | +5, all of it the `906–924` drift |
| `src/menu.rs` | 48 | 48 | — |
| `src/launch.rs` | 15 | 15 | — |
| `src/main.rs` | 21 | 21 | — |
| `src/desktop_shutdown.rs` | 19 | **17** | −2, the `499–500` race, now covered |
| `src/future_login.rs` | 3 | 3 | — |
| `src/desktop_shutdown/coordinator.rs` | 8 | 8 | — |
| `src/macos_power.rs` | 10 | 10 | — |
| `src/runtime.rs` | 4 | 4 | — |
| `src/instance.rs` | 5 | 5 | — |
| `src/host_environment.rs` | 0 | 0 | — |
| `src/bin/futureos-headless.rs` | 1 | 1 | — |
| **total** | **242** | **245** | **+3** |

**The +3 is exactly the two documented instabilities and nothing else** — proved per line
and per record, not by arithmetic (`/tmp/c5-uniondiff.py` compares both union reports
file-by-file: same record sets everywhere, and these are the only zero-vs-non-zero flips):

* `src/lib.rs`: `906`, `910`, `917`, `923`, `924` — pinned `2` → this run `0`. The
  `tauri_plugin_single_instance` callback body, which only a second-instance launch can
  execute; §S2.4/§S3.6.5/§S4.2 already recorded it drifting both ways (7–12 `DA:0`
  lines). **Not this unit's regression, and not claimed as a gain when it is covered.**
* `src/desktop_shutdown.rs`: `499`, `500` — pinned `0` → this run `1`. The §S2.8.4
  `freeze_tasks` race, in the other direction this time (a sibling `perform_stop` thread
  must not win between this fixture's `cancel_test_tasks()` and its `spawn`).

**No third line moved**, so this is not a new finding: the method reproduces, and its
spread is the two lines of instability the record already carries. Also unchanged: the
gui leg ran **complete** this time (2059 tests, one peer failure) and the headless leg
ran 1636 tests with 6 failures — the port-8022 `remote_peer` family, plus
`agent_bridge::import::tests::import_one_spawns_cwd_write_back_and_logs_its_failure` and
`skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`, all
peer-owned.

## S5.2 The tripwire against the new STALE half (supervisor's tool change)

`reconcile.py`'s STALE half now resolves a ledger row against every file the report
*measured*, instead of skipping a file whose uncovered set has become empty
(commit `5da793b6`, "the STALE half skipped fully-covered files (F.35)"). That is exactly
this unit's situation for `src/host_environment.rs`, which reached **0 uncovered lines** in
Segment 2, so the check was re-run deliberately:

| invocation | MISSING | STALE | COVERED | ABSENT | verdict |
|---|---|---|---|---|---|
| `--section "Segment 4"` (authoritative) | **0** | *(no block printed)* | **0** | **0** | clean — exit 0, non-vacuous (242 claims parsed) |
| `--section "Segment 5"` | 74 | *(no block printed)* | — | — | **expected and by design**: this section carries no ledger, only the reproduction and the proposal, so there is nothing to satisfy `mentioned`; the tool prints its vacuity WARNING ("0 parsed as ledger claims") rather than a silent 0. The reproduction's own delta is accounted for in §S5.1, line by line |
| `--section "Segment 3"` | 0 | 13 | 13 | 0 | expected: §S3.4's index describes the 255-line revision |
| `--section "Segment 2"` | 24 | 55 | 28 | 27 | expected: superseded ledger |
| `--section "8. Per-line index"` | 57 | 133 | 45 | 88 | expected: the first revision's 318-row index |
| whole document (no `--section`) | 0 | 170 | 68 | 102 | the union of the four superseded slices |

**The `host_environment.rs` transition, re-read by hand as the steering asks.** The only
row that ever claimed a line in that file is §8's index row
(`| `src/host_environment.rs:28` | 4.11 | …`), and the new check now reports it under
COVERED — correctly: this unit's own fixture
(`a_host_without_a_resolvable_cli_still_starts_logging`, §4.11 row 9 / §S2.3 row 9)
covers that line, and §S2.3 already lists it as **covered**. So it is a
superseded-section row whose claim was retired by a later segment, not a live claim that
disagrees with the report. No row in §S2.6, §S3.4 or §S4.4 disagrees with the report
(Segment 4's slice prints no STALE block at all).

**Why the superseded rows stay in the document.** They are the citation base of the
campaign: the review unit (`todo_e79acc9f0c8d`, REV-TAURI-SHELL) is tasked with
adversarially falsifying "the 289 waiver tags … in … section 4", and Segment 2/3's prose
refers to `§4.11` row numbers and §8 rows. Deleting them would make those citations
unfalsifiable and would *hide* the history this record exists to keep. The honest fix is
what the header now does: a revision map, a per-section revision note on §2/§4/§8, this
table of what a whole-document run prints, and the single authoritative invocation above.
The pairing is right in every one of these runs — all five use the same report the record
pins, and the per-line detail agrees.

**The pairing rule this exposed, and why the row stays.** Re-running the STALE half with
this segment's reproduction report paired against **Segment 4's ledger** — a legitimate
thing for a reviewer to do, since both describe the same source — makes the check print
`STALE: 2, COVERED: 2` for exactly `desktop_shutdown.rs:499` and `:500`: the flaky pair
that this run measured *covered* (§S5.1) and the pinned run measured `DA:0`. Two things
follow, and neither is a reason to touch the row or the report:

* **the authoritative pairing is the report the ledger was written against**
  (`/tmp/tauri-shell-lcov-seg4union.info`), which gives 242 / 242 / MISSING 0 / no STALE
  block — the row is *correct* for that report, and §S4.2 already records that these two
  lines are conditionally executed and appear in either state;
* **`reconcile.py`'s exit code is 0 even when it prints STALE rows** (it exits non-zero
  only on MISSING), so a wrapper that reads only the exit code cannot see this class of
  disagreement — read the printed block, and read it against the pinned artifact. That is
  the general form of the "it fires on your record" case the steering describes: check the
  pairing first, and fix the row only when the row is wrong *for its own revision*.

## S5.3 PLAN §4b proposal: a helper process that runs the real runtime on a real main thread

**Not landed, not approved — this is the decision the supervisor asked for.** The 156
lines in S4.1 (98) + S4.2 (58) are waived for one mechanism: `muda`/AppKit require the
process's **main thread**, and no libtest body is ever the main thread (§6.2, measured:
`pthread_main_np()` = 0 inside a `#[test]` even with `--test-threads=1`; and
`Builder::<Wry>::build` aborts with `muda::MenuChild can only be created on the main
thread` in a test body — §S2.8.5).

**Fact (b) — profiles from a spawned process do merge: already measured.** §S4.7.2: the
eight child-only lines of Segment 4's fixtures are non-zero in the merged report, and
cargo-llvm-cov's `LLVM_PROFILE_FILE` pattern carries the pid
(`src-tauri-<pid>-<hash>_0.profraw`), so parent and child each write their own file and the
merge sums them. A helper *process* is therefore not blocked on plumbing.

**Fact (a) — a real main thread does get through the build here: measured in this
segment.** An uninstrumented, freshly built `futureos` was launched with an isolated
`HOME`/`FUTURE_HOME` (`/tmp/tau-c5-wry-probe.sh`):

* the process **stayed alive for 30 s** — the event loop is running, so
  `Builder::<Wry>::build(context)` returned `Ok` and `app.run(…)` was entered;
* `$HOME/.future/app/logs/desktop.log` exists and its **first line** is
  `FutureOS: host starting version=…` — that is `host_environment::start_logging()`,
  which `run_with` calls *after* `.build(context)` and before `app.run`
  (`src/lib.rs:1195–1198`), so the log line is evidence that the statement a libtest body
  aborts on was executed;
* the lines after it are the `Ready` arm's work in the same process (the embedded Agent
  binding an isolated socket/db and becoming ready), i.e. the real handle reached
  `start_agent_supervision` too;
* no window appeared (the configured main window is `"visible": false`), and it had to be
  `SIGKILL`ed — a raw Wry app cannot be asked to quit from outside.

**Arithmetic — an estimate from the source, labelled as such.** A helper process that runs
the same `run_with` on a real runtime would execute, from the current 242:

| cluster | lines | why a real-runtime process executes it |
|---|---|---|
| `menu.rs` `build_macos_menu` body | **48** | called by the `.setup` hook (`lib.rs:1005`) — this is the single largest block the main-thread rule currently waives |
| `lib.rs` `run_with` head and tail — 980, 984–986, 991–993, 1195, 1196, 1198, 1204, 1205 | ~12 | the body of the function the helper is running |
| `lib.rs` `.setup` hook body — 1004–1008, 1010, 1011, 1031–1040 | ~17 | the hook body, including the menu install, the dock icon and the window show/geometry calls |
| `lib.rs` `size_main_window_to_screen` → `apply_main_window_geometry` arguments — 209–215 | 7 | needs a real window and a real monitor (the two existing tests take the early-return arms) |
| `lib.rs` entry points — 888, 889, 890 | 3 | reached via `main.rs` → `gui::run()` |
| **subtotal** | **≈ 87** | |
| *stays*: the plugin/menu/window hook **bodies** (907–975, ~45), the emitters (811, 842), `macos_power.rs`'s three notification bodies (10), `future_login`/`instance`/`coordinator`/`launcher` families, 1227/1252/1253/1256, 1267–1279, 1398–1411 | ~155 of the 156 | they need a *second instance*, a menu click, a window close request, a real sleep/wake/power-off notification, or the peer-owned remote state |

So the proposal is worth **≈ 87 lines (242 → ≈ 155)**, not the whole 156: a helper process
supplies the main thread, not the events. **The number is an estimate and the record does
not claim it as measured** — it is exactly what the first instrumented helper run would
settle.

**The open engineering item, which is also the scope question.** For those lines to be
*counted*, the helper must exit **normally**: LLVM writes the profile at exit, and the
probe had to be `SIGKILL`ed because a raw app has no external quit path. A usable helper
therefore has to be a **new binary** — a purpose-built `src/bin/…` whose `main()` runs
`run_with` on the real runtime and then calls `app.exit(0)` after a bounded window (the
`Exit` arm would then be exercised as a bonus; `ExitRequested` stays unreachable —
`tauri::ExitRequestApi` has private fields, `app.rs:81-84`/`:2554`). The alternative — an
env-var-triggered self-exit inside `run_with` — is a **production** change with a
test-only branch, which §4b's "smallest change / behaviour-identical" bar does not favour.
`src/bin/` is **outside this todo's declared write set**, so this is a supervisor decision,
and it also needs a host answer: the probe succeeded on this macOS session, and nothing yet
says a CI runner or a Linux container can start a Wry runtime at all.

**Equivalence argument, for the record.** No production line changes under this proposal:
the helper is test infrastructure that calls the same `run_with` entry point. The
observable differences of running it are (1) a real window is created (configured
invisible), (2) a nested Agent starts against the helper's own `HOME`/`FUTURE_HOME` (the
probe showed it binding an isolated socket and database, so the user's agent is untouched),
and (3) the process must be exited by the helper itself rather than by a user. If the
supervisor wants the yield, the smallest form is one new `src/bin/` file plus an
integration test that spawns it under the instrumented build.

## S5.4 Gates (PLAN §5.1)

Run in `desktop/src-tauri`, same environment as §S5.1, on this revision — no source
changed in this segment, so these are the state of the tree the reproduction measured:

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **1** | 2 diffs, **both in a peer-owned file**: `src/agent_bridge/import.rs:1670`, `src/agent_bridge/observer.rs:1772` (peer units' in-flight test code). **This unit's file is clean**: `rustfmt --edition 2021 --config skip_children=true --check src/lib.rs` → **0** |
| clippy (gui, all targets) | `cargo clippy --all-targets -- -D warnings` | **101** | **one** error, in a peer-owned file: `src/store/threads.rs:1341` (`clippy::cloned_ref_to_slice_refs`, inside `mod tests` — test code, `src/store/**` is not in this unit's write set). No error is reported in any file of this unit |
| clippy (headless, all targets) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **101** | the same single peer-owned error |
| clippy (production only, both flavours) | `cargo clippy --lib -- -D warnings` / `… --no-default-features --features headless --lib -- -D warnings` | **0** / **0** | clean — the red above is test code in a peer's module, not production |
| lib suite, gui | the leg in §S5.1 | **101** | `2059 passed; 1 failed; 12 ignored` |
| lib suite, headless | the leg in §S5.1 | **101** | `1636 passed; 6 failed; 12 ignored` — `remote_peer::runtime_tests::*` (port 8022) plus two peer-owned fixtures |
| this unit's fixtures (all 12) | `cargo test --lib -- --test-threads=1 <the 12 names>` | **0** | 12 passed / 0 failed |

The two fmt diffs and the one clippy error are **outside this unit's scope** and are
reported rather than fixed (PLAN §2b: never format a peer's in-flight file). Both were
introduced by peer units between Segment 4 and now: the same commands were exit 0 for fmt
and exit 0 for clippy at the end of Segment 4.

## S5.5 Findings

### S5.5.1 The reproduction's spread is the two instabilities, and the method is otherwise exact

Every file and every line of the scope matched the pinned measurement except the two
named lines of instability, and `+5 − 2 = +3` is the whole difference. That is the
strongest form this unit can give the "is the number trustworthy" question: the second run
does not just land near 242, it lands *on* the two lines the record predicted would move.
A third run is still nice-to-have, not owed.

### S5.5.2 A target can be absent from a per-flavour report, and the tool says so

`covlines.py total … --under desktop/src-tauri/src/bin/futureos-headless.rs` against the
**gui-only** report exits with `ERROR: no per-line records found — wrong file, or --under
does not match this report (a zero denominator reads as 100 %)`. Correct behaviour and
worth knowing: that bin is `headless`-only, so the gui flavour never builds it and only the
union has its record. A per-file loop that ignored the tool's exit code would have read
that file as 100 % covered from the gui leg alone.

### S5.5.3 The gui leg can be complete or truncated run-to-run; only the report tells you

Segment 4's single-report run lost the gui leg's late data while exiting 0 (§S4.7.3); this
segment's gui leg ran to completion (2059 tests). The lesson holds: per-flavour render,
then union, and read the *report's* per-file numbers rather than the leg's exit code.

### S5.5.4 The probe's own side effects, named

Running the real app — even with an isolated `HOME`/`FUTURE_HOME` — starts a nested
embedded Agent (isolated socket and database, threaded in-process, so nothing is left
orphaned by the `SIGKILL`), and the app never exits by itself. Anyone repeating §S5.3's
probe should run it the same way (isolated home, bounded wait, then kill) and delete the
probe directory; `/tmp/c5-wry-probe/` was removed after the measurement.

## S5.6 Reproduction

```bash
 # 1. this segment's reproduction (§S5.1)
bash /tmp/tau-c5-repro.sh                       # ~26 min, both legs + union
python3 /tmp/c5-uniondiff.py                    # pinned vs this run, per line
 # 2. the tripwire (§S5.2): the authoritative invocation is
 #    reconcile.py --section "Segment 4"  (0 MISSING, no STALE block)
 # 3. the main-thread probe (§S5.3) — behavioural, uninstrumented, ~30 s
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo build --bin futureos                      # exit 0
bash /tmp/tau-c5-wry-probe.sh                   # then: check desktop.log + liveness
 # 4. the gates (§S5.4)
cargo fmt --check ; rustfmt --edition 2021 --config skip_children=true --check src/lib.rs
cargo clippy --all-targets -- -D warnings ; cargo clippy --lib -- -D warnings
```

# Segment 6 — the auto-connect pass's deferred and failed arms are covered; a silent process-exit bug in this unit's own registration test is found, reproduced and fixed

Status: **measured, source changed** — the measured revision is `src/lib.rs` with sha256
prefix `0ad1243b10085cb5` (last written 01:19:29 local; the re-run that produced this
segment's numbers started 01:31:00, so nothing changed under the measurement).

| residual, this unit's 12 scope files | Segment 5 (same source) | **this segment** |
|---|---|---|
| total | 245 | **268** |
| of which production | 242 (pinned ledger) | **235** |
| test-region artifacts / flaky | 3 | **33** |

`268 = 235 production + 28 test-region attribution records + 3 flaky/attribution (S6.9 2 +
S6.14 2 − 1 recovered brace)`. The production residual went **down by 7** — the two arms
this segment's fixtures drive — and the arithmetic is line-by-line in §S6.2. **Zero**
production line moved from covered to `DA:0` (checked per line against Segment 5's report,
not by totals).

What changed, in one paragraph: two fixtures drive `spawn_remote_auto_connect`'s spawned
pass to both of its remaining outcomes — a *deferred* start (the platform refuses the
credential refresh) and a *failed* start (the persisted desktop NKey cannot be read) — with
no new seam and no use of `install_fake_state`, because the state that decides those arms is
home state, not an installed bridge. Getting a *trustworthy* measurement out of this took a
second fix, and it is the more valuable half: the registration test this unit added in
Segment 3 builds the single-instance plugin, whose socket is `/tmp/<identifier>_si.sock`,
and the plugin answers a live listener on that socket with **`std::process::exit(0)`**
(`tauri-plugin-single-instance 2.4.3`, `platform_impl/macos.rs:23-31`). The mock context's
identifier is the empty string, so *every* process on this host shares `/tmp/_si.sock`: a
second concurrent run silently kills the first one mid-suite — exit status 0, no test-result
line, and every test after that point unrun. This segment's first gui leg died exactly that
way (927 of 2074 tests; `store`, `runtime` and `tasks` never ran; the report was partial),
which is a **silently wrong coverage number with exit code 0**. The test now uses a
process-unique identifier, and §S6.7.1 has the two-process control that proves both the bug
and the fix.

## S6.1 Run identity and the exact measurement

Both flavours, per-flavour reports, then the union (Segment 5's method; the campaign's
single-report recipe remains unusable — §S4.7.3):

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH \
       CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup \
       HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home \
       CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-shell
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder   # exit 0

cargo llvm-cov clean --manifest-path Cargo.toml                                     # exit 0
 # gui leg — FIRST attempt, /tmp/tau-c4b-measure.sh, and its number was DISCARDED:
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
 #   → exit 0, but the lib target printed 927 of 2074 tests and no `test result` line:
 #     /tmp/_si.sock collision (§S6.7.1). Report /tmp/tauri-shell-lcov-c4b-gui.info is
 #     PARTIAL (lib.rs 210, store/threads.rs 12 → 693) and is not used anywhere below.
 # headless leg — unaffected (it does not compile `mod gui`):
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml \
    --no-default-features --features headless -- --test-threads=1
 #   → exit 0 (green): 1679 tests, /tmp/tauri-shell-lcov-c4b-headless.info
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tauri-shell-lcov-c4b-headless.info
 #   → exit 0
 # gui leg — RE-RUN with the socket fix in place (/tmp/tau-c4b-gui-rerun.sh):
cargo llvm-cov clean --manifest-path Cargo.toml                                    # exit 0
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
 #   → exit 101: 2056 passed / 6 failed / 12 ignored, complete (all six named in §S6.5)
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tauri-shell-lcov-c4b2-gui.info
 #   → exit 0
python3 /tmp/lcov-union.py --out /tmp/tauri-shell-lcov-c4b2union.info \
    /tmp/tauri-shell-lcov-c4b2-gui.info /tmp/tauri-shell-lcov-c4b-headless.info             # exit 0
```

The artifact of record is **`/tmp/tauri-shell-lcov-c4b2union.info`**. Evidence for the
seven lines this segment covers, straight out of that report (`covlines.py detail`):

| row (in the report) | what it is | union | gui-only |
|---|---|---|---|
| the `if !enabled` block's closing brace — `lib.rs:1398` | brace of the gate | **2** | 2 |
| `crate::runtime::spawn(async {` — `lib.rs:1400` | the pass's spawn | **2** | 2 |
| `match remote::start(...).await {` — `lib.rs:1401` | the dispatch | **2** | 2 |
| the `Ok(status) if matches!(phase, Ready)` **guard** — `lib.rs:1402` | evaluated, not taken | **1** | 1 |
| the **deferred** `eprintln!` — `lib.rs:1405` | fixture 1's target | **1** | 1 |
| the **failed** `eprintln!` — `lib.rs:1409` | fixture 2's target | **1** | 1 |
| the closure's `});` — `lib.rs:1411` | pass closes | **2** | 2 |
| the ready arm's `eprintln!` — `lib.rs:1403` | see §S6.7.2 (decision) | 0 | 0 |

(Line references are written as prose above on purpose: a table row whose *first* cell is
``path:line`` is read by `reconcile.py` as a ledger claim — the same mechanical rule that
makes the per-line index a ledger — and a table of *covered* lines in that shape would show
up as a false STALE.)

and the pass's own report was observed, not just its line: both new tests passed in the
same instrumented leg (`/tmp/tau-c4b-gui-rerun.log`), and each asserts the message its arm
prints plus the **absence** of the other arm's message, so an input that slipped into the
wrong arm fails the test rather than passing it.

## S6.2 Before and after, per file

Each column is `covlines.py total … --under <file>` against that segment's report:

| file | Segment 4 (pinned) | Segment 5 | **this segment** | note |
|---|---|---|---|---|
| `src/lib.rs` | 108 | 113 | **134** | −7 covered, **+28** new test-region records (§S6.7.4) |
| `src/menu.rs` | 48 | 48 | 48 | — |
| `src/launch.rs` | 15 | 15 | 15 | — |
| `src/main.rs` | 21 | 21 | 21 | — |
| `src/desktop_shutdown.rs` | 19 | 17 | **19** | the `499–500` flake, back (§S6.7.5) |
| `src/future_login.rs` | 3 | 3 | 3 | — |
| `src/desktop_shutdown/coordinator.rs` | 8 | 8 | 8 | — |
| `src/macos_power.rs` | 10 | 10 | 10 | — |
| `src/runtime.rs` | 4 | 4 | 4 | — |
| `src/instance.rs` | 5 | 5 | 5 | — |
| `src/host_environment.rs` | 0 | 0 | 0 | — |
| `src/bin/futureos-headless.rs` | 1 | 1 | 1 | — |
| **total** | **242** | **245** | **268** | |

Measured (non-zero) lines in the same scope: **3687 → 3839 → 3968** (`lib.rs` 971 → 1123 →
1335; the +212 is this segment's two fixtures plus the registration test's new comment).

**The arithmetic, line by line.** Against Segment 5's report (the same source, so the
comparison is exact rather than line-drift-confused):

* **newly covered: 7** — `lib.rs` 1398, 1400, 1401, 1402, 1405, 1409, 1411 (§S6.1);
* **newly uncovered: 0** — no line anywhere in `lib.rs` moved from covered to `DA:0`
  (verified as a set difference, not a total);
* **+28** — new `DA:0` records that did not exist in Segment 5's report (§S6.7.4);
* **+2** — `desktop_shutdown.rs` 499–500 (§S6.7.5);
* check: `245 − 7 + 28 + 2 = 268`.

On the pinned basis the **production** residual is therefore `242 − 7 = 235`, and the
difference between the two baselines (242 vs 245) is the drift/flake pair Segment 5
documented.

## S6.3 The two fixtures

**(1) The deferred arm** — `an_auto_connect_whose_platform_is_unreachable_is_reported_as_deferred`.
Signed in with a key (`set_future_login("sekret", "http://127.0.0.1:1")`, so the gate's
`future_api_key().is_ok()` holds), `auto_connect_remote` on, and a **pairing on disk** with
a valid NKey seed. `remote::start` then runs and its credential refresh cannot be sent:
the platform is a closed port, the send error is a transport failure, `error_code` maps it
to `network`, and `start_failure` turns it into a *status* — the arm's own
`FutureOS remote auto-connect: deferred (…)` is observed, and the test also asserts the
failure message is **absent**.

**(2) The failed arm** — `an_auto_connect_with_an_unreadable_pairing_key_is_reported_as_failed`.
Same state, with the one field a *local* fault lives in: `nkey_seed` set to
`not-a-nkey-seed`. `refresh_bridge_jwt` reads the seed **before** any request, so
`nkeys::KeyPair::from_seed` fails with an uncategorized `AppError::Message`; `start_failure`
returns it as `Err`, and the pass reports
`FutureOS remote auto-connect failed: read desktop NKey …`. The deferred message must be
absent.

Neither fixture needs `remote::test_support::install_fake_state` — as the TAU-C3 handoff
predicted, both arms sit behind `remote::start`'s *result* (home state decides it), not
behind an installed bridge. Both are offline (a refused connection and a seed parse) and
deterministic: the pair runs in 0.14 s uninstrumented and passed in the instrumented leg.

## S6.4 The residual (268 lines) — categories

| group | lines | category | what places the line there |
|---|---|---|---|
| S6.1 the real Tauri/Wry application | 103 | `unreachable-in-this-environment` | `Builder::<Wry>::build` creates the Wry event loop and Tauri's default menu, and `muda` only builds menus on the main thread (§6.2). Hook bodies additionally need real window/menu/IPC events |
| S6.2 macOS main-thread-only — `menu.rs` (48) + `macos_power.rs` (10) | 58 | `unreachable-in-this-environment` | `muda` asserts `NSThread::isMainThread`; the power hooks are AppKit/IOKit notifications |
| S6.3 the launcher *process* bodies | 36 | `unreachable-in-this-environment` | they spawn a real GUI Desktop and wait for its answer (§4.4) |
| S6.4 the Linux-only helper success arm | 1 | `platform-unmeasured` | `bin/futureos-headless.rs:11` compiles only on Linux |
| S6.5 `cfg(not(test))`-excluded blocks | 11 | `unreachable-by-construction` | behind `#[cfg(all(feature = "gui", not(test)))]`; the test cfg removes them |
| S6.6 the `flock` re-acquire window | 5 | `unreachable-in-this-environment` | `instance.rs` 164–168 needs a lock taken inside the re-acquire window |
| S6.7 client-stack arms | 3 | `unreachable-in-this-environment` | `future_login.rs` 354–355, 482: §6.5's measured result |
| S6.8 never-evaluated / fixture-only arms | 8 | `unreachable-by-construction` | all in **test** code, where the fixture's own design is the invariant: `lib.rs` 1727 (the burst producer's early `return`), 1906–1907 (`Failed::flush`'s `Ok`), `runtime.rs` 248–252, `desktop_shutdown.rs:501` |
| S6.9 region llvm-cov attributes to a block-end brace | 2 | `attribution-artifact` | `desktop_shutdown.rs` 263 and 460 close blocks whose bodies are covered (260–262, 457–459); the third line of Segment 5's group (`lib.rs:1398`) is now covered |
| S6.10 the **ready** arm's body | 1 | `unreachable-in-this-environment` | `lib.rs:1403`. The guard line above it is now covered; reaching the body needs `remote::start` to return `Ready`, i.e. a live NATS broker (`connect_nats`) — and on the way `bind_web_listener()` binds the fixed `WEB_PORT = 8022` (remote/diagnostics.rs:153). §S6.7.2 names the route this unit would take next (both halves exist as `pub(crate)` test support) — it is named, **not** claimed |
| S6.11 the shutdown deadline arm | 3 | `unreachable-in-this-environment` | `desktop_shutdown.rs` 256–258. The state half is available (`install_fake_state`, whose own doc names this row); the **budget** half is not constructible from this unit's write set: `Operation::remaining(2 s)` is `min(2 s, deadline − now)`, the only entry point with a custom budget is `fn begin_with_budget`, private to `desktop_shutdown/coordinator.rs` (a peer-owned file — not in this unit's write set), and with the production `TOTAL_BUDGET = 20 s` the arm cannot fire because `stop_gracefully`'s awaited publish is itself capped at `DISCONNECT_NOTICE_TIMEOUT = 750 ms` (`remote/diagnostics.rs:158-162`), and the earlier *callers* wait is bounded by `remaining(3 s)`. Measured in this segment's report: 250–252 and 255 are covered (25 hits) and 256–258 are `DA:0` |
| S6.12 an OS thread-creation refusal | 7 | `unreachable-in-this-environment` | `coordinator.rs` 165–166, 183–187: the two `std::thread::Builder::spawn` error arms |
| S6.13 zero-count `DA` records inside passing tests | 28 | `attribution-artifact` | a **new** class this segment measured: lines of three *passing* tests (`the_ready_event…`, `the_exit_event…`, and one `process_state…` assert continuation) that this build gives `DA:0` records — comments, blank lines, `);` and `}` continuations. Evidence: those tests report `ok` in the same leg, and Segment 5's report had **0** such lines ≥ 2350 (the code is identical; only its position in the file changed). Not unexecuted behaviour, and §S6.7.4 says why the count moved |
| S6.14 the `freeze_tasks` race pair | 2 | `unreachable-in-this-environment` | `desktop_shutdown.rs` 499–500: the body of the future `runtime_failure_still_drops_background_work_and_blocks_retry` spawns. Covered in isolation, `DA:0` in a full serialized run — a *sibling* fixture's non-abortable `perform_stop` thread can call `runtime::freeze_tasks()` between this fixture's `cancel_test_tasks()` and its `spawn`, which makes `runtime::spawn` drop the future unpolled. The test passes either way (§S2.8.4's probe is the reproduction) |
| **total** | **268** | | |

**Arithmetic:** `unreachable-in-this-environment` = 103 + 58 + 36 + 5 + 3 + 1 + 3 + 7 + 2
= 218; `unreachable-by-construction` = 11 + 8 = 19; `platform-unmeasured` = 1;
`attribution-artifact` = 2 + 28 = 30. `218 + 19 + 1 + 30 = 268` ✓, and the per-line index
below has exactly 268 rows.

Checked mechanically before hand-off (PLAN §5.2) — §S6.6 has the invocation and its exit
code.

### Per-line index of this segment's residual (268 rows)

| line | group |
|---|---|
| `src/lib.rs:209` | S6.1 |
| `src/lib.rs:210` | S6.1 |
| `src/lib.rs:211` | S6.1 |
| `src/lib.rs:212` | S6.1 |
| `src/lib.rs:213` | S6.1 |
| `src/lib.rs:214` | S6.1 |
| `src/lib.rs:215` | S6.1 |
| `src/lib.rs:474` | S6.1 |
| `src/lib.rs:475` | S6.1 |
| `src/lib.rs:476` | S6.1 |
| `src/lib.rs:477` | S6.1 |
| `src/lib.rs:478` | S6.1 |
| `src/lib.rs:811` | S6.1 |
| `src/lib.rs:842` | S6.1 |
| `src/lib.rs:888` | S6.1 |
| `src/lib.rs:889` | S6.1 |
| `src/lib.rs:890` | S6.1 |
| `src/lib.rs:906` | S6.1 |
| `src/lib.rs:907` | S6.1 |
| `src/lib.rs:908` | S6.1 |
| `src/lib.rs:909` | S6.1 |
| `src/lib.rs:910` | S6.1 |
| `src/lib.rs:916` | S6.1 |
| `src/lib.rs:917` | S6.1 |
| `src/lib.rs:919` | S6.1 |
| `src/lib.rs:920` | S6.1 |
| `src/lib.rs:921` | S6.1 |
| `src/lib.rs:923` | S6.1 |
| `src/lib.rs:924` | S6.1 |
| `src/lib.rs:932` | S6.1 |
| `src/lib.rs:933` | S6.1 |
| `src/lib.rs:934` | S6.1 |
| `src/lib.rs:935` | S6.1 |
| `src/lib.rs:936` | S6.1 |
| `src/lib.rs:937` | S6.1 |
| `src/lib.rs:941` | S6.1 |
| `src/lib.rs:942` | S6.1 |
| `src/lib.rs:943` | S6.1 |
| `src/lib.rs:945` | S6.1 |
| `src/lib.rs:950` | S6.1 |
| `src/lib.rs:955` | S6.1 |
| `src/lib.rs:956` | S6.1 |
| `src/lib.rs:957` | S6.1 |
| `src/lib.rs:958` | S6.1 |
| `src/lib.rs:960` | S6.1 |
| `src/lib.rs:962` | S6.1 |
| `src/lib.rs:963` | S6.1 |
| `src/lib.rs:964` | S6.1 |
| `src/lib.rs:965` | S6.1 |
| `src/lib.rs:967` | S6.1 |
| `src/lib.rs:968` | S6.1 |
| `src/lib.rs:969` | S6.1 |
| `src/lib.rs:970` | S6.1 |
| `src/lib.rs:971` | S6.1 |
| `src/lib.rs:974` | S6.1 |
| `src/lib.rs:975` | S6.1 |
| `src/lib.rs:980` | S6.1 |
| `src/lib.rs:984` | S6.1 |
| `src/lib.rs:985` | S6.1 |
| `src/lib.rs:986` | S6.1 |
| `src/lib.rs:991` | S6.1 |
| `src/lib.rs:992` | S6.1 |
| `src/lib.rs:993` | S6.1 |
| `src/lib.rs:995` | S6.1 |
| `src/lib.rs:1004` | S6.1 |
| `src/lib.rs:1005` | S6.1 |
| `src/lib.rs:1006` | S6.1 |
| `src/lib.rs:1007` | S6.1 |
| `src/lib.rs:1008` | S6.1 |
| `src/lib.rs:1010` | S6.1 |
| `src/lib.rs:1011` | S6.1 |
| `src/lib.rs:1031` | S6.1 |
| `src/lib.rs:1032` | S6.1 |
| `src/lib.rs:1033` | S6.1 |
| `src/lib.rs:1034` | S6.1 |
| `src/lib.rs:1035` | S6.1 |
| `src/lib.rs:1036` | S6.1 |
| `src/lib.rs:1037` | S6.1 |
| `src/lib.rs:1038` | S6.1 |
| `src/lib.rs:1039` | S6.1 |
| `src/lib.rs:1040` | S6.1 |
| `src/lib.rs:1195` | S6.1 |
| `src/lib.rs:1196` | S6.1 |
| `src/lib.rs:1198` | S6.1 |
| `src/lib.rs:1204` | S6.1 |
| `src/lib.rs:1205` | S6.1 |
| `src/lib.rs:1206` | S6.1 |
| `src/lib.rs:1227` | S6.1 |
| `src/lib.rs:1252` | S6.1 |
| `src/lib.rs:1253` | S6.1 |
| `src/lib.rs:1256` | S6.1 |
| `src/lib.rs:1267` | S6.1 |
| `src/lib.rs:1268` | S6.1 |
| `src/lib.rs:1270` | S6.1 |
| `src/lib.rs:1271` | S6.1 |
| `src/lib.rs:1272` | S6.1 |
| `src/lib.rs:1273` | S6.1 |
| `src/lib.rs:1275` | S6.1 |
| `src/lib.rs:1276` | S6.1 |
| `src/lib.rs:1277` | S6.1 |
| `src/lib.rs:1278` | S6.1 |
| `src/lib.rs:1279` | S6.1 |
| `src/lib.rs:1403` | S6.10 |
| `src/lib.rs:1727` | S6.8 |
| `src/lib.rs:1906` | S6.8 |
| `src/lib.rs:1907` | S6.8 |
| `src/lib.rs:2513` | S6.13 |
| `src/lib.rs:2514` | S6.13 |
| `src/lib.rs:2516` | S6.13 |
| `src/lib.rs:2521` | S6.13 |
| `src/lib.rs:2522` | S6.13 |
| `src/lib.rs:2525` | S6.13 |
| `src/lib.rs:2533` | S6.13 |
| `src/lib.rs:2539` | S6.13 |
| `src/lib.rs:2540` | S6.13 |
| `src/lib.rs:2550` | S6.13 |
| `src/lib.rs:2551` | S6.13 |
| `src/lib.rs:2554` | S6.13 |
| `src/lib.rs:2559` | S6.13 |
| `src/lib.rs:2560` | S6.13 |
| `src/lib.rs:2571` | S6.13 |
| `src/lib.rs:2572` | S6.13 |
| `src/lib.rs:2579` | S6.13 |
| `src/lib.rs:2580` | S6.13 |
| `src/lib.rs:2584` | S6.13 |
| `src/lib.rs:2585` | S6.13 |
| `src/lib.rs:2589` | S6.13 |
| `src/lib.rs:2592` | S6.13 |
| `src/lib.rs:2593` | S6.13 |
| `src/lib.rs:2594` | S6.13 |
| `src/lib.rs:2599` | S6.13 |
| `src/lib.rs:2602` | S6.13 |
| `src/lib.rs:2603` | S6.13 |
| `src/lib.rs:2613` | S6.13 |
| `src/menu.rs:26` | S6.2 |
| `src/menu.rs:27` | S6.2 |
| `src/menu.rs:28` | S6.2 |
| `src/menu.rs:29` | S6.2 |
| `src/menu.rs:31` | S6.2 |
| `src/menu.rs:32` | S6.2 |
| `src/menu.rs:33` | S6.2 |
| `src/menu.rs:34` | S6.2 |
| `src/menu.rs:35` | S6.2 |
| `src/menu.rs:36` | S6.2 |
| `src/menu.rs:37` | S6.2 |
| `src/menu.rs:38` | S6.2 |
| `src/menu.rs:39` | S6.2 |
| `src/menu.rs:40` | S6.2 |
| `src/menu.rs:41` | S6.2 |
| `src/menu.rs:42` | S6.2 |
| `src/menu.rs:43` | S6.2 |
| `src/menu.rs:44` | S6.2 |
| `src/menu.rs:45` | S6.2 |
| `src/menu.rs:46` | S6.2 |
| `src/menu.rs:47` | S6.2 |
| `src/menu.rs:48` | S6.2 |
| `src/menu.rs:49` | S6.2 |
| `src/menu.rs:51` | S6.2 |
| `src/menu.rs:52` | S6.2 |
| `src/menu.rs:53` | S6.2 |
| `src/menu.rs:54` | S6.2 |
| `src/menu.rs:55` | S6.2 |
| `src/menu.rs:56` | S6.2 |
| `src/menu.rs:57` | S6.2 |
| `src/menu.rs:58` | S6.2 |
| `src/menu.rs:59` | S6.2 |
| `src/menu.rs:61` | S6.2 |
| `src/menu.rs:62` | S6.2 |
| `src/menu.rs:63` | S6.2 |
| `src/menu.rs:65` | S6.2 |
| `src/menu.rs:66` | S6.2 |
| `src/menu.rs:67` | S6.2 |
| `src/menu.rs:68` | S6.2 |
| `src/menu.rs:69` | S6.2 |
| `src/menu.rs:70` | S6.2 |
| `src/menu.rs:72` | S6.2 |
| `src/menu.rs:73` | S6.2 |
| `src/menu.rs:74` | S6.2 |
| `src/menu.rs:75` | S6.2 |
| `src/menu.rs:76` | S6.2 |
| `src/menu.rs:77` | S6.2 |
| `src/menu.rs:78` | S6.2 |
| `src/launch.rs:330` | S6.3 |
| `src/launch.rs:331` | S6.3 |
| `src/launch.rs:332` | S6.3 |
| `src/launch.rs:333` | S6.3 |
| `src/launch.rs:334` | S6.3 |
| `src/launch.rs:338` | S6.3 |
| `src/launch.rs:339` | S6.3 |
| `src/launch.rs:340` | S6.3 |
| `src/launch.rs:341` | S6.3 |
| `src/launch.rs:342` | S6.3 |
| `src/launch.rs:343` | S6.3 |
| `src/launch.rs:344` | S6.3 |
| `src/launch.rs:345` | S6.3 |
| `src/launch.rs:346` | S6.3 |
| `src/launch.rs:347` | S6.3 |
| `src/main.rs:9` | S6.3 |
| `src/main.rs:29` | S6.3 |
| `src/main.rs:33` | S6.3 |
| `src/main.rs:34` | S6.3 |
| `src/main.rs:38` | S6.3 |
| `src/main.rs:39` | S6.3 |
| `src/main.rs:41` | S6.3 |
| `src/main.rs:42` | S6.3 |
| `src/main.rs:43` | S6.3 |
| `src/main.rs:44` | S6.3 |
| `src/main.rs:46` | S6.3 |
| `src/main.rs:47` | S6.3 |
| `src/main.rs:48` | S6.3 |
| `src/main.rs:51` | S6.3 |
| `src/main.rs:58` | S6.3 |
| `src/main.rs:70` | S6.3 |
| `src/main.rs:71` | S6.3 |
| `src/main.rs:72` | S6.3 |
| `src/main.rs:73` | S6.3 |
| `src/main.rs:74` | S6.3 |
| `src/main.rs:75` | S6.3 |
| `src/desktop_shutdown.rs:221` | S6.5 |
| `src/desktop_shutdown.rs:222` | S6.5 |
| `src/desktop_shutdown.rs:223` | S6.5 |
| `src/desktop_shutdown.rs:256` | S6.11 |
| `src/desktop_shutdown.rs:257` | S6.11 |
| `src/desktop_shutdown.rs:258` | S6.11 |
| `src/desktop_shutdown.rs:263` | S6.9 |
| `src/desktop_shutdown.rs:267` | S6.5 |
| `src/desktop_shutdown.rs:268` | S6.5 |
| `src/desktop_shutdown.rs:269` | S6.5 |
| `src/desktop_shutdown.rs:270` | S6.5 |
| `src/desktop_shutdown.rs:271` | S6.5 |
| `src/desktop_shutdown.rs:272` | S6.5 |
| `src/desktop_shutdown.rs:273` | S6.5 |
| `src/desktop_shutdown.rs:274` | S6.5 |
| `src/desktop_shutdown.rs:460` | S6.9 |
| `src/desktop_shutdown.rs:499` | S6.14 |
| `src/desktop_shutdown.rs:500` | S6.14 |
| `src/desktop_shutdown.rs:501` | S6.8 |
| `src/future_login.rs:354` | S6.7 |
| `src/future_login.rs:355` | S6.7 |
| `src/future_login.rs:482` | S6.7 |
| `src/desktop_shutdown/coordinator.rs:50` | S6.1 |
| `src/desktop_shutdown/coordinator.rs:165` | S6.12 |
| `src/desktop_shutdown/coordinator.rs:166` | S6.12 |
| `src/desktop_shutdown/coordinator.rs:183` | S6.12 |
| `src/desktop_shutdown/coordinator.rs:184` | S6.12 |
| `src/desktop_shutdown/coordinator.rs:185` | S6.12 |
| `src/desktop_shutdown/coordinator.rs:186` | S6.12 |
| `src/desktop_shutdown/coordinator.rs:187` | S6.12 |
| `src/macos_power.rs:21` | S6.2 |
| `src/macos_power.rs:22` | S6.2 |
| `src/macos_power.rs:23` | S6.2 |
| `src/macos_power.rs:24` | S6.2 |
| `src/macos_power.rs:26` | S6.2 |
| `src/macos_power.rs:27` | S6.2 |
| `src/macos_power.rs:29` | S6.2 |
| `src/macos_power.rs:30` | S6.2 |
| `src/macos_power.rs:31` | S6.2 |
| `src/macos_power.rs:32` | S6.2 |
| `src/runtime.rs:248` | S6.8 |
| `src/runtime.rs:249` | S6.8 |
| `src/runtime.rs:251` | S6.8 |
| `src/runtime.rs:252` | S6.8 |
| `src/instance.rs:164` | S6.6 |
| `src/instance.rs:165` | S6.6 |
| `src/instance.rs:166` | S6.6 |
| `src/instance.rs:167` | S6.6 |
| `src/instance.rs:168` | S6.6 |
| `src/bin/futureos-headless.rs:11` | S6.4 |

## S6.5 The gates (PLAN §5.1) and the suites

Run in `desktop/src-tauri`, same environment as §S6.1, on the measured revision:

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **1** | **peer-owned only**, and the set moves as peers edit: first observation (before the measurement) 2 diffs — `src/agent_bridge/import.rs:1670`, `src/agent_bridge/observer.rs:1772`; re-run after the measurement 3 diffs — `src/agent_bridge/reconciliation.rs:830, 897, 939`. **This unit's file is clean in both**: `rustfmt --edition 2021 --config skip_children=true --check src/lib.rs` → **0** |
| clippy (gui) | `cargo clippy --all-targets -- -D warnings` | **101** | **one peer-owned error**, also moving: first observation `src/store/threads.rs:1341` (`clippy::cloned_ref_to_slice_refs`, in `mod tests`), re-run `src/agent_bridge/reconciliation.rs:813` (`unused imports: Reply and get_state_payload`). No diagnostic points at any file of this unit |
| clippy (all targets, peer lint allowed) | `cargo clippy --all-targets -- -D warnings -A unused_imports` | **0** | with only that one peer lint relaxed, the whole crate — **including this unit's new test code** — is clean. This is the check that makes the claim above precise: the red is a peer's lint, not a hidden one of ours |
| clippy (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **101** | the same single peer-owned error pattern |
| clippy (production only) | `cargo clippy --lib -- -D warnings` | **0** | clean in both flavours — the red above is always a peer's *test* code |
| lib suite, gui (the measured leg) | §S6.1, re-run | **101** | `2056 passed; 6 failed; 12 ignored` — **all six named below, none in this unit's scope** |
| lib suite, headless | §S6.1 | **0** | **green** (1679 tests) — the flavour that does not compile this segment's code |
| this unit's 14 fixtures | `cargo test --lib -- --test-threads=1 <the 14 names>` | **0** | 14 passed (12 from earlier segments + the 2 new ones; the new pair alone: 2 passed in 0.14 s) |

**The six gui failures, named and classified (§5.3):**

* five are the `remote_peer` family, all panicking at `src/remote_peer/testing.rs:30`
  (`the test web port never freed`) after
  `remote: web client bind failed [LC002]: web server bind on port 8022 failed: Address
  already in use (os error 48)` —
  `runtime_tests::{labels_round_trip_through_the_list_and_are_clearable,
  only_the_host_that_failed_is_retried,
  opening_a_conversation_moves_the_event_lane_and_closing_it_moves_back,
  pairing_a_link_connects_and_reports_the_host}` and
  `transfer_tests::a_chunk_write_without_a_connection_is_refused`. The port is fixed
  (`WEB_PORT = 8022`, `remote/diagnostics.rs:153`) and other crate-test processes on this
  host hold it (FLAKES.md's inventory). `src/remote_peer/**` is outside this unit's write
  set.
* one is `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`
  (`src/skills_bootstrap.rs:225`), the `cli_tool`-discovery class FLAKES.md item 27
  records (a `PATH`-replacing fixture under load). Peer-owned.

**Direction argument:** the failures are outside this unit's scope, they are the two
families the record already documents, and they cannot hide a regression of mine — none of
the six touches `gui::startup_paths_tests::*`, and the arm coverage this segment claims is
verified *per line* in the report from that same leg (§S6.1), not inferred from the exit
code. The headless flavour of the same revision is green; the gui flavour's red is the
host's fixed port plus a peer fixture.

## S6.6 PLAN §5.2 tripwire (per-line, mechanical)

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-c4b2union.info \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 6" --label tauri-shell-seg6
```

## S6.7 Findings

### S6.7.1 A silent `std::process::exit(0)` in this unit's own registration test (found, reproduced, fixed)

**The bug.** `the_desktop_registration_builds_for_any_runtime` (added in Segment 3) builds
the plugin chain, which includes `tauri_plugin_single_instance`. That plugin's `setup`
computes `socket_path(config, …)` = `/tmp/<identifier>_si.sock` and calls
`notify_singleton(&socket)`; when a **live** listener answers, it runs
`std::process::exit(0)` (`tauri-plugin-single-instance 2.4.3`,
`src/platform_impl/macos.rs:23-31`). The mock context's identifier is the empty string
(`tauri::test::mock_context` sets `identifier: Default::default()`), so the socket is
`/tmp/_si.sock` — **shared by every process on the host**.

**How it behaved.** This segment's first gui leg reported `GUI_RUN_EXIT=0` yet its log
stopped after `gui::…the_desktop_registration_builds_for_any_runtime` with **no
`test result` line**, and `store`, `runtime`, `tasks`, `instance` and `host_environment`
never ran at all (grep count 0 for every one of them). The same signature reproduced in an
uninstrumented run: `running 394 tests`, 58 printed, exit 0, no summary. A coverage report
over such a leg is *partial* (`lib.rs` 210 instead of ~113; `store/threads.rs` 12 → 693) —
i.e. a **silently wrong number with a green exit code**, the failure direction the
campaign's report guards cannot catch.

**The reproduction, and the control that proves the fix** (`/tmp/si-control.sh`): the same
test run by two processes concurrently.

| binary | run A | run B |
|---|---|---|
| before the fix | 29 tests printed, **no result line**, exit 0 | 33 tests, full result line |
| after the fix | **35 tests, full result line** | **35 tests, full result line** |

and the sockets the two runs used are now distinct:
`/tmp/com_futureos_test_<pid>_si.sock` for each, versus the single shared `/tmp/_si.sock`
before.

**The fix** (one line in the test, no production change): the mock context gets a
process-unique identifier —
`context.config_mut().identifier = format!("com.futureos.test.{}", std::process::id());` —
with a comment recording why the value is load-bearing. Consequences worth keeping: any
future test that builds a plugin chain with a mock context must set an identifier, and a
*stale* socket is handled by the plugin itself (its `Err(ConnectionRefused)` arm claims the
socket), so only a live listener is dangerous.

### S6.7.2 Decision (a) — the ready arm (`lib.rs:1403`): registered, with the route named

Kept **registered** this segment — it is `DA:0` in the measured report — and this segment
did produce the evidence for how it *would* be covered, which the successor should use
rather than re-deriving:

* `crate::remote::test_support::{FakeNats, MockPlatform}` are `pub(crate)` (both
  constructors `pub(crate) async fn start()`), so a **shell test can drive them**, unlike
  `RemoteState`/`SUPERVISOR` which are `pub(in crate::remote)`;
* the path is `establish()` → `MockPlatform::respond_refresh(nats_url)` (the peer's own
  `refresh_bridge_jwt_success_and_failures` uses exactly this) → `connect_nats` against a
  `FakeNats` broker → `status.phase = Ready` → the arm;
* the two facts that make it robust: `web_client_enabled()` is `true` in test builds, and a
  failed `bind_web_listener` is **non-fatal** (a `web_bind` warning, the bridge still
  reports `Ready`) — so a peer holding port 8022 does not break the fixture.

This is a proposal with its parts named, not a claim: the arm stays in the ledger until a
fixture runs.

### S6.7.3 Decision (b) — the shutdown deadline arm (`desktop_shutdown.rs:256-258`): registered, knob owners named

Kept **registered**, and the reason is now measured rather than argued: in this segment's
report `desktop_shutdown.rs` 250–252 and 255 carry **25** hits each while 256–258 are
`DA:0` — i.e. the real shutdown path runs every test that reaches it and the deadline arm
never fires. It cannot fire under the production budget: `operation.remaining(2 s)` is
`min(2 s, deadline − now)` with `TOTAL_BUDGET = 20 s`, the earlier *callers* wait is itself
bounded by `remaining(3 s)`, and `stop_gracefully`'s awaited publish is capped at
`DISCONNECT_NOTICE_TIMEOUT = 750 ms` (`remote/diagnostics.rs:158-162`). Turning the budget
knob needs a *smaller* budget, and the only entry point is
`fn begin_with_budget` — private to `desktop_shutdown/coordinator.rs`, which is **not in
this unit's declared write set** (`lib.rs`, `desktop_shutdown.rs`, the record). So the arm
stays with two named owners: (i) whoever owns `coordinator.rs` making a budget entry point
reachable to `desktop_shutdown.rs` (e.g. `pub(super) fn begin_with_budget`), or (ii) a
fixture in `src/remote/**` that makes the presence publish block past the window.

### S6.7.4 A new artifact class, measured: 28 zero-count records inside passing tests

Segment 5's report had **0** `DA:0` lines ≥ 2350 in `lib.rs`; this segment's has **28**, all
inside three tests that report `ok` in the same leg (comments, blank lines, `);`/`}` and one
assert continuation). The code of those tests is unchanged — only its position moved,
because this segment's fixtures were inserted above them. So the count is a function of the
file's *layout*, not of behaviour, and it is classified `attribution-artifact` (the same
category as the two block-end braces, with the same kind of evidence: the neighbouring
lines are covered and the enclosing test passes). Consequence for the campaign's numbers: a
per-file residual can grow when *test* code is added above existing tests, without a single
production line becoming unreachable — this is why §S6.2's arithmetic is stated per line
rather than as a total.

### S6.7.5 The flaky pair, back

`desktop_shutdown.rs` 499–500 are `DA:0` in this run (Segment 5 measured them covered) —
the §S2.8.4 `freeze_tasks` race, in the same direction Segment 4 saw. It is the third
observation of the same pair flipping, and the probe in §S2.8.4 still reproduces it in
isolation. Nothing in this segment touches that fixture.

## S6.8 Reproduction

```bash
 # 1. the measurement (§S6.1): per-flavour legs + union
bash /tmp/tau-c4b-measure.sh          # gui leg (discarded) + headless leg + union
bash /tmp/tau-c4b-gui-rerun.sh        # the gui leg that counts (fix in place)
python3 /tmp/lcov-union.py --out /tmp/tauri-shell-lcov-c4b2union.info \
    /tmp/tauri-shell-lcov-c4b2-gui.info /tmp/tauri-shell-lcov-c4b-headless.info
 # 2. the child-process-exit bug and its control (§S6.7.1)
bash /tmp/si-control.sh               # two concurrent runs; pre-fix one dies with exit 0
 # 3. this unit's new fixtures, uninstrumented and fast
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --lib -- --test-threads=1 an_auto_connect_whose_platform_is_unreachable_is_reported_as_deferred
cargo test --lib -- --test-threads=1 an_auto_connect_with_an_unreadable_pairing_key_is_reported_as_failed
 # 4. the tripwire (§S6.6) and the gates (§S6.5)
```

# Appendix S7 — the helper-process gate: three measured facts, and why the family is still blocked

**No source change and no new coverage in this part of the record.** It is written so the
next reader (or the supervisor deciding TAU-C6's gate) does not re-run the probes, and so
the *estimate* in §S5.3 is not mistaken for a measurement. The authoritative ledger is
still §S6.4 (268 rows, 0 open); the tripwire invocation for it is unchanged and re-run:

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-c4b2union.info \
    --scope desktop/src-tauri/src/lib.rs … --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 6" --label …   # exit 0
 #   → uncovered lines=268  mentioned=268  MISSING=0  (no STALE block)
```

## S7.1 The three facts

**Fact 1 — a real main thread really runs this app (re-confirmed).** A spawned, instrumented
*gui* build of `futureos`, with an isolated `HOME`/`FUTURE_HOME`, ran for 15 s in its event
loop and reached `FutureOS: embedded Agent ready` in its log (i.e. `Builder::<Wry>::build`
returned `Ok`, `app.run` was entered, and the `Ready` arm's work — supervision against an
isolated agent socket/database — ran). No window appeared (the configured window is
`visible: false`). Artifacts: `/tmp/c5-continuous2/app.log`, script
`/tmp/c5-continuous2/`'s probe (`/tmp/c5-continuous-probe2.sh`).

**Fact 2 — the profile is written only at a *normal* exit, so an external kill loses it.**
The same gui run, with `LLVM_PROFILE_FILE=<scratch>/app-%m-%p.profraw`, was polled once per
second for 15 s and then killed with `SIGKILL`:

* while alive: **no** `.profraw` at any poll (15 samples);
* after `SIGKILL`: still **no** `.profraw`.

Control that the pattern and the runtime are correct: a **headless-flavour** build of the
same crate (which exits by itself immediately) under the *same* pattern wrote a profraw of
49568 bytes at once. So the difference is the exit, not the environment or the specifier:
LLVM's runtime flushes in its `atexit` handler
(`__llvm_profile_register_write_file_atexit` is in the toolchain's
`libprofiler_builtins`), and neither `SIGKILL` nor a default `SIGTERM`/`SIGINT` runs it.

**Fact 3 — `%m` is the merged-binary-id specifier, not continuous sync.** The "capture the
app's profile without a new helper binary" route I tried was to enable LLVM's continuous
sync mode through the profile filename. It does not work this way in this toolchain:
the runtime here does expose `___llvm_profile_enable_continuous_mode` /
`ContinuouslySyncProfile` (found by `strings` in
`…/lib/rustlib/aarch64-apple-darwin/lib/libprofiler_builtins-*.rlib`), but a 15-second-lived
process with `%m` in the pattern wrote nothing (Fact 2), and the *expanded* name shows what
`%m` actually is — a build hash:
`app-2094133869635435188_0-<pid>.profraw`. Continuous mode would need the program to call
that runtime entry point itself (Rust exposes no flag for it), which is a change inside the
app, not in the measurement.

**Fact 4 (source) — no signal-based graceful exit exists for the gui app.** The crate's
only signal handling is `headless/mod.rs`'s Windows `ctrl_c`/`ctrl_close` and
`terminal/pty.rs`'s signals to *child shells*; nothing in `run_with`'s path turns a signal
into `app.exit()`. So the outside world has no way to make this app exit normally.

## S7.2 What follows for the gate

An external process **can** drive the 156-line family (Fact 1) but **cannot keep** what it
earns (Facts 2–4) unless the process ends normally, and the only normal end available is the
app asking itself to stop. That is exactly the `src/bin/` helper §S5.3 proposed (a bounded
window, then `app.exit(0)`), and it is still **outside this unit's declared write set**.

Decision requested (TAU-C6 item 0), unchanged in substance and now cheaper to take:

* **(i) grant one new file under `desktop/src-tauri/src/bin/`** for this unit — the proposal
  then becomes measurable and §S5.3's ~87-line estimate can be falsified by a measurement
  (its clusters: `menu.rs` 48 called from the setup hook, `run_with`'s head/tail and the
  hook body, the geometry calls that need a real window/monitor, the three entry points);
  **or**
* **(iii) — RULED, and this is what happened:** the supervisor granted the driver as a
  **`harness = false` test target** (`desktop/src-tauri/tests/gui_main_thread_drive.rs`),
  *not* the `src/bin/` form §S5.3 proposed (commit `6e3710416`, "grant the driver as a
  harness=false test target (iii), not src/bin"). The reasoning is recorded there and it is
  better than the proposal: a `harness = false` target gets a real `fn main()`, so the test
  body itself IS the main thread, while a `[[bin]]` would have shipped a stray binary. The
  work is TAU-C8.
* **(ii) rule the family waived** — it already is, with mechanisms, and §S6.4 closes with
  **0 open**, so nothing in the campaign's arithmetic depends on the decision.

One caveat about the probes, recorded because it nearly produced a wrong conclusion:
they ran a **peer unit's** instrumented binary
(`target/cov-tauri-appdata/llvm-cov-target/debug/futureos`), read-only, with the profile
pattern pointed at my own `/tmp` scratch. That path changed flavour under the probe — a gui
build at 01:54:39 (the one Fact 1 measured) and the unit's *headless* build minutes later
(same path, 86 MB, which is why an identical invocation then reported "no graphical
interface"). A peer's in-flight target dir is a moving target; a conclusion drawn from it
must name the moment it was taken, and the same probes re-run against this unit's own
instrumented build would be the belt-and-braces confirmation if the gate is granted.

# Segment 7 — the auto-connect pass's **ready** arm is covered; the shell's residual falls to 238 with zero open

Status: **measured, source changed** — `src/lib.rs` gained one fixture
(`an_auto_connect_that_connects_is_reported_as_connected`). The measured artifact is
**`/tmp/tauri-shell-lcov-c7union.info`** (per-flavour reports unioned, §S7.1). The scope
residual is **238**, down from **268** (Segment 6) and below the **242** that Segment 4
recorded as this unit's floor. Every line is waived with a mechanism; **0 open**.

## S7.1 Run identity and the exact measurement

Same two-leg recipe as §S6.1 (the single-report step stays unusable, §S4.7.3), run as
`/tmp/tau-c7-measure.sh` → `/tmp/tau-c7-measure.log` (18:03:41→18:30:19 UTC):

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH \
       CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup \
       HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home \
       CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-shell
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder   # exit 0
cargo llvm-cov clean --manifest-path Cargo.toml                                     # exit 0
 # gui leg
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml -- --test-threads=1
 #   → exit 101: lib 2063 passed / 8 failed / 12 ignored, finished 609.67s
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tauri-shell-lcov-c7-gui.info
 #   → exit 0
 # headless leg
cargo llvm-cov --no-report --no-fail-fast -j 4 --manifest-path Cargo.toml \
    --no-default-features --features headless -- --test-threads=1
 #   → exit 0: lib 1650 passed / 0 failed / 12 ignored (GREEN)
cargo llvm-cov report --manifest-path Cargo.toml --lcov --output-path /tmp/tauri-shell-lcov-c7-headless.info
 #   → exit 0
python3 /tmp/lcov-union.py --out /tmp/tauri-shell-lcov-c7union.info \
    /tmp/tauri-shell-lcov-c7-gui.info /tmp/tauri-shell-lcov-c7-headless.info             # exit 0
```

**The completeness check §S7.3 requires, run on the gui leg's log:**
`grep -c "test store::threads"` = **28** (the tail modules ran) and the lib target printed
its `test result` line — so this leg is not an §S6.7.1 silent-exit truncation.

**The line this segment covers** — `lib.rs:1403`, the ready arm's body — with its
neighbours in the report:

| line | union | Segment 6 |
|---|---|---|
| `lib.rs:1398` (gate brace) | 3 | 2 |
| `lib.rs:1400` (`spawn`) | 3 | 2 |
| `lib.rs:1401` (the `match`) | 3 | 2 |
| `lib.rs:1402` (the `Ready` guard) | 2 | 1 |
| **`lib.rs:1403` (the ready `eprintln!`)** | **1** | **0** |
| `lib.rs:1405` (deferred) | 1 | 1 |
| `lib.rs:1409` (failed) | 1 | 1 |
| `lib.rs:1411` (closure end) | 3 | 2 |

## S7.2 The fixture, and what it actually required

`an_auto_connect_that_connects_is_reported_as_connected` (in `gui::startup_paths_tests`)
runs the pass against a live fixture, using the peers' own test support — no new seam, no
production change:

```rust
let platform = MockPlatform::start().await;               // pub(crate) test support
let nats = FakeNats::start().await;                       // in-process broker
crate::remote::test_support::sign_in(platform.url());     // the gate's key + the refresh's host
let creds = crate::remote::test_support::pairing_creds("shell-ready");
crate::remote::pairing::save_creds(&creds);               // the gate also needs a pairing
platform.respond_refresh(nats.url());                     // establish() → refresh_bridge_jwt
store::update_app_settings(auto_connect_remote: Some(true));
spawn_remote_auto_connect();
// …poll crate::remote::status() until Ready, then assert and stop()
```

**The recipe is not the one the todo predicted, and the difference is the gate.** The pass
only runs at all when all three gate conditions hold, including
`remote::pairing::load_creds().is_some()` — so a "fresh pairing" fixture
(`respond_pair_code`, no stored creds) returns at the gate and never reaches the arm. My
first attempt did exactly that and failed with `last status … phase: Stopped`; the fixture
that works persists a pairing and answers the **refresh** (`respond_refresh(nats.url())`),
which is also what the peers' `re_pair_revokes_the_stored_pairing_before_issuing_an_invitation`
does. Recorded because the todo's own text ("persist pairing creds…", then
`respond_refresh`) was right and my first version deviated from it.

**The assertion is the pass's effect, not the message** (libtest captures `eprintln!` in a
passing run — §S4.7.2): after the pass, the process-global supervisor holds a bridge in
`RemotePhase::Ready`, running the pairing the fixture persisted, pointed at the broker the
refresh returned, with the pairing still on disk. Nothing else in that test starts a bridge,
so the state is attributable; and the test calls `crate::remote::stop()` and waits (bounded,
10 s) until port 8022 is bindable again, because a leaked bridge would both break the
peers' fixtures (they assert an empty `SUPERVISOR.state`) and add to the suite's port-8022
flake. The `FutureOS remote auto-connect: connected` line was observed directly in the
debugging run (it is only printed when the test fails, because of that capture).

## S7.3 Before and after, per file

| file | Segment 6 | **this segment** | note |
|---|---|---|---|
| `src/lib.rs` | 134 | **105** | −29 (§S7.4) |
| `src/menu.rs` | 48 | 48 | — |
| `src/launch.rs` | 15 | 15 | — |
| `src/main.rs` | 21 | 21 | — |
| `src/desktop_shutdown.rs` | 19 | **18** | the `499–500` pair: one line recovered this run |
| `src/future_login.rs` | 3 | 3 | — |
| `src/desktop_shutdown/coordinator.rs` | 8 | 8 | — |
| `src/macos_power.rs` | 10 | 10 | — |
| `src/runtime.rs` | 4 | 4 | — |
| `src/instance.rs` | 5 | 5 | — |
| `src/host_environment.rs` | 0 | 0 | — |
| `src/bin/futureos-headless.rs` | 1 | 1 | — |
| **total** | **268** | **238** | |

Measured lines in the scope: 3968 → 3900 (`lib.rs` 1335 → 1234 records: adding a test above
the others **re-attributed** the region map again — see §S7.4).

## S7.4 The arithmetic of −30, line by line

Against Segment 6's report for the same file (so this is a set comparison, not a total):

* **+1 covered: the arm** (`lib.rs:1403`) — this segment's work;
* **+16 covered: test-region attribution records** that Segment 6 listed as `DA:0`
  (`lib.rs` 2514, 2516, 2522, 2533, 2550, 2551, 2554, 2580, 2585, 2589, 2592, 2593, 2599,
  2602, 2603) — they were `attribution-artifact` rows (S6.13), never unexecuted behaviour,
  and the layout change flipped them;
* **+12 retired**: former zero-count records that no longer exist as separate regions
  (1335 → 1234 records for the same file) — the same artifact class, in the direction that
  removes rows rather than covering them;
* **+1 recovered: `desktop_shutdown.rs`'s flaky pair** (one of 499/500 covered this run);
* **0 newly uncovered** — nothing in the production region (`lib.rs` < 2100) and nothing in
  the test region moved from covered to `DA:0` in this run.

`268 − 30 = 238`. **The honest reading of the −29 in `lib.rs` is therefore: 1 line of
this unit's work, 16+12 lines of attribution artifact that the file's own layout
controls, and the rest the flaky pair** — which is exactly why §S6.2/S7.4 exist: this
unit's *production* number is what a reviewer should hold onto (242 pinned → 238, and
the 4-line difference is the arm plus the two instabilities, not a code change).

## S7.5 The residual (238 lines) — categories

| group | lines | category | what places the line there |
|---|---|---|---|
| S7.1 the real Tauri/Wry application | 103 | `unreachable-in-this-environment` | `Builder::<Wry>::build` + the hook bodies (§6.2) |
| S7.2 macOS main-thread-only — `menu.rs` 48 + `macos_power.rs` 10 | 58 | `unreachable-in-this-environment` | `muda` asserts the main thread; AppKit/IOKit notifications |
| S7.3 the launcher *process* bodies | 36 | `unreachable-in-this-environment` | they spawn a real GUI Desktop (§4.4) |
| S7.4 the Linux-only helper success arm | 1 | `platform-unmeasured` | `bin/futureos-headless.rs:11` |
| S7.5 `cfg(not(test))`-excluded blocks | 11 | `unreachable-by-construction` | `desktop_shutdown.rs` 221–223, 267–274 |
| S7.6 the `flock` re-acquire window | 5 | `unreachable-in-this-environment` | `instance.rs` 164–168 |
| S7.7 client-stack arms | 3 | `unreachable-in-this-environment` | `future_login.rs` 354–355, 482 |
| S7.8 never-evaluated / fixture-only arms | 8 | `unreachable-by-construction` | `lib.rs` 1727, 1906–1907; `runtime.rs` 248–252; `desktop_shutdown.rs:501` — all in **test** code |
| S7.9 region llvm-cov attributes to a block-end brace | 1 | `attribution-artifact` | `desktop_shutdown.rs:263` or `:460` (the other three braces of this class are covered this run) |
| S7.11 the shutdown deadline arm | 3 | `unreachable-in-this-environment` | `desktop_shutdown.rs` 256–258 — §S7.6 |
| S7.12 an OS thread-creation refusal | 7 | `unreachable-in-this-environment` | `coordinator.rs` 165–166, 183–187 |
| S7.14 the `freeze_tasks` race pair | 2 | `unreachable-in-this-environment` | `desktop_shutdown.rs` 499–500 — §S2.8.4's probe; **500 is covered this run**, 499 is not |
| **total** | **238** | | |

**Arithmetic:** `unreachable-in-this-environment` = 103 + 58 + 36 + 5 + 3 + 3 + 7 + 2 =
217; `unreachable-by-construction` = 11 + 8 = 19; `platform-unmeasured` = 1;
`attribution-artifact` = 1. `217 + 19 + 1 + 1 = 238` ✓, and the per-line index below has
exactly 238 rows.

Two groups from Segment 6 are **gone**: S6.10 (the ready arm — covered now) and S6.13 (the
28 test-region attribution records — 16 covered, 12 retired as regions).

Checked mechanically before hand-off (PLAN §5.2) — §S7.8 has the invocation and its exit
code.

### Per-line index of this segment's residual (238 rows)

| line | group |
|---|---|
| `src/lib.rs:209` | S7.1 |
| `src/lib.rs:210` | S7.1 |
| `src/lib.rs:211` | S7.1 |
| `src/lib.rs:212` | S7.1 |
| `src/lib.rs:213` | S7.1 |
| `src/lib.rs:214` | S7.1 |
| `src/lib.rs:215` | S7.1 |
| `src/lib.rs:474` | S7.1 |
| `src/lib.rs:475` | S7.1 |
| `src/lib.rs:476` | S7.1 |
| `src/lib.rs:477` | S7.1 |
| `src/lib.rs:478` | S7.1 |
| `src/lib.rs:811` | S7.1 |
| `src/lib.rs:842` | S7.1 |
| `src/lib.rs:888` | S7.1 |
| `src/lib.rs:889` | S7.1 |
| `src/lib.rs:890` | S7.1 |
| `src/lib.rs:906` | S7.1 |
| `src/lib.rs:907` | S7.1 |
| `src/lib.rs:908` | S7.1 |
| `src/lib.rs:909` | S7.1 |
| `src/lib.rs:910` | S7.1 |
| `src/lib.rs:916` | S7.1 |
| `src/lib.rs:917` | S7.1 |
| `src/lib.rs:919` | S7.1 |
| `src/lib.rs:920` | S7.1 |
| `src/lib.rs:921` | S7.1 |
| `src/lib.rs:923` | S7.1 |
| `src/lib.rs:924` | S7.1 |
| `src/lib.rs:932` | S7.1 |
| `src/lib.rs:933` | S7.1 |
| `src/lib.rs:934` | S7.1 |
| `src/lib.rs:935` | S7.1 |
| `src/lib.rs:936` | S7.1 |
| `src/lib.rs:937` | S7.1 |
| `src/lib.rs:941` | S7.1 |
| `src/lib.rs:942` | S7.1 |
| `src/lib.rs:943` | S7.1 |
| `src/lib.rs:945` | S7.1 |
| `src/lib.rs:950` | S7.1 |
| `src/lib.rs:955` | S7.1 |
| `src/lib.rs:956` | S7.1 |
| `src/lib.rs:957` | S7.1 |
| `src/lib.rs:958` | S7.1 |
| `src/lib.rs:960` | S7.1 |
| `src/lib.rs:962` | S7.1 |
| `src/lib.rs:963` | S7.1 |
| `src/lib.rs:964` | S7.1 |
| `src/lib.rs:965` | S7.1 |
| `src/lib.rs:967` | S7.1 |
| `src/lib.rs:968` | S7.1 |
| `src/lib.rs:969` | S7.1 |
| `src/lib.rs:970` | S7.1 |
| `src/lib.rs:971` | S7.1 |
| `src/lib.rs:974` | S7.1 |
| `src/lib.rs:975` | S7.1 |
| `src/lib.rs:980` | S7.1 |
| `src/lib.rs:984` | S7.1 |
| `src/lib.rs:985` | S7.1 |
| `src/lib.rs:986` | S7.1 |
| `src/lib.rs:991` | S7.1 |
| `src/lib.rs:992` | S7.1 |
| `src/lib.rs:993` | S7.1 |
| `src/lib.rs:995` | S7.1 |
| `src/lib.rs:1004` | S7.1 |
| `src/lib.rs:1005` | S7.1 |
| `src/lib.rs:1006` | S7.1 |
| `src/lib.rs:1007` | S7.1 |
| `src/lib.rs:1008` | S7.1 |
| `src/lib.rs:1010` | S7.1 |
| `src/lib.rs:1011` | S7.1 |
| `src/lib.rs:1031` | S7.1 |
| `src/lib.rs:1032` | S7.1 |
| `src/lib.rs:1033` | S7.1 |
| `src/lib.rs:1034` | S7.1 |
| `src/lib.rs:1035` | S7.1 |
| `src/lib.rs:1036` | S7.1 |
| `src/lib.rs:1037` | S7.1 |
| `src/lib.rs:1038` | S7.1 |
| `src/lib.rs:1039` | S7.1 |
| `src/lib.rs:1040` | S7.1 |
| `src/lib.rs:1195` | S7.1 |
| `src/lib.rs:1196` | S7.1 |
| `src/lib.rs:1198` | S7.1 |
| `src/lib.rs:1204` | S7.1 |
| `src/lib.rs:1205` | S7.1 |
| `src/lib.rs:1206` | S7.1 |
| `src/lib.rs:1227` | S7.1 |
| `src/lib.rs:1252` | S7.1 |
| `src/lib.rs:1253` | S7.1 |
| `src/lib.rs:1256` | S7.1 |
| `src/lib.rs:1267` | S7.1 |
| `src/lib.rs:1268` | S7.1 |
| `src/lib.rs:1270` | S7.1 |
| `src/lib.rs:1271` | S7.1 |
| `src/lib.rs:1272` | S7.1 |
| `src/lib.rs:1273` | S7.1 |
| `src/lib.rs:1275` | S7.1 |
| `src/lib.rs:1276` | S7.1 |
| `src/lib.rs:1277` | S7.1 |
| `src/lib.rs:1278` | S7.1 |
| `src/lib.rs:1279` | S7.1 |
| `src/lib.rs:1727` | S7.8 |
| `src/lib.rs:1906` | S7.8 |
| `src/lib.rs:1907` | S7.8 |
| `src/menu.rs:26` | S7.2 |
| `src/menu.rs:27` | S7.2 |
| `src/menu.rs:28` | S7.2 |
| `src/menu.rs:29` | S7.2 |
| `src/menu.rs:31` | S7.2 |
| `src/menu.rs:32` | S7.2 |
| `src/menu.rs:33` | S7.2 |
| `src/menu.rs:34` | S7.2 |
| `src/menu.rs:35` | S7.2 |
| `src/menu.rs:36` | S7.2 |
| `src/menu.rs:37` | S7.2 |
| `src/menu.rs:38` | S7.2 |
| `src/menu.rs:39` | S7.2 |
| `src/menu.rs:40` | S7.2 |
| `src/menu.rs:41` | S7.2 |
| `src/menu.rs:42` | S7.2 |
| `src/menu.rs:43` | S7.2 |
| `src/menu.rs:44` | S7.2 |
| `src/menu.rs:45` | S7.2 |
| `src/menu.rs:46` | S7.2 |
| `src/menu.rs:47` | S7.2 |
| `src/menu.rs:48` | S7.2 |
| `src/menu.rs:49` | S7.2 |
| `src/menu.rs:51` | S7.2 |
| `src/menu.rs:52` | S7.2 |
| `src/menu.rs:53` | S7.2 |
| `src/menu.rs:54` | S7.2 |
| `src/menu.rs:55` | S7.2 |
| `src/menu.rs:56` | S7.2 |
| `src/menu.rs:57` | S7.2 |
| `src/menu.rs:58` | S7.2 |
| `src/menu.rs:59` | S7.2 |
| `src/menu.rs:61` | S7.2 |
| `src/menu.rs:62` | S7.2 |
| `src/menu.rs:63` | S7.2 |
| `src/menu.rs:65` | S7.2 |
| `src/menu.rs:66` | S7.2 |
| `src/menu.rs:67` | S7.2 |
| `src/menu.rs:68` | S7.2 |
| `src/menu.rs:69` | S7.2 |
| `src/menu.rs:70` | S7.2 |
| `src/menu.rs:72` | S7.2 |
| `src/menu.rs:73` | S7.2 |
| `src/menu.rs:74` | S7.2 |
| `src/menu.rs:75` | S7.2 |
| `src/menu.rs:76` | S7.2 |
| `src/menu.rs:77` | S7.2 |
| `src/menu.rs:78` | S7.2 |
| `src/launch.rs:330` | S7.3 |
| `src/launch.rs:331` | S7.3 |
| `src/launch.rs:332` | S7.3 |
| `src/launch.rs:333` | S7.3 |
| `src/launch.rs:334` | S7.3 |
| `src/launch.rs:338` | S7.3 |
| `src/launch.rs:339` | S7.3 |
| `src/launch.rs:340` | S7.3 |
| `src/launch.rs:341` | S7.3 |
| `src/launch.rs:342` | S7.3 |
| `src/launch.rs:343` | S7.3 |
| `src/launch.rs:344` | S7.3 |
| `src/launch.rs:345` | S7.3 |
| `src/launch.rs:346` | S7.3 |
| `src/launch.rs:347` | S7.3 |
| `src/main.rs:9` | S7.3 |
| `src/main.rs:29` | S7.3 |
| `src/main.rs:33` | S7.3 |
| `src/main.rs:34` | S7.3 |
| `src/main.rs:38` | S7.3 |
| `src/main.rs:39` | S7.3 |
| `src/main.rs:41` | S7.3 |
| `src/main.rs:42` | S7.3 |
| `src/main.rs:43` | S7.3 |
| `src/main.rs:44` | S7.3 |
| `src/main.rs:46` | S7.3 |
| `src/main.rs:47` | S7.3 |
| `src/main.rs:48` | S7.3 |
| `src/main.rs:51` | S7.3 |
| `src/main.rs:58` | S7.3 |
| `src/main.rs:70` | S7.3 |
| `src/main.rs:71` | S7.3 |
| `src/main.rs:72` | S7.3 |
| `src/main.rs:73` | S7.3 |
| `src/main.rs:74` | S7.3 |
| `src/main.rs:75` | S7.3 |
| `src/desktop_shutdown.rs:221` | S7.5 |
| `src/desktop_shutdown.rs:222` | S7.5 |
| `src/desktop_shutdown.rs:223` | S7.5 |
| `src/desktop_shutdown.rs:256` | S7.11 |
| `src/desktop_shutdown.rs:257` | S7.11 |
| `src/desktop_shutdown.rs:258` | S7.11 |
| `src/desktop_shutdown.rs:267` | S7.5 |
| `src/desktop_shutdown.rs:268` | S7.5 |
| `src/desktop_shutdown.rs:269` | S7.5 |
| `src/desktop_shutdown.rs:270` | S7.5 |
| `src/desktop_shutdown.rs:271` | S7.5 |
| `src/desktop_shutdown.rs:272` | S7.5 |
| `src/desktop_shutdown.rs:273` | S7.5 |
| `src/desktop_shutdown.rs:274` | S7.5 |
| `src/desktop_shutdown.rs:460` | S7.9 |
| `src/desktop_shutdown.rs:499` | S7.14 |
| `src/desktop_shutdown.rs:500` | S7.14 |
| `src/desktop_shutdown.rs:501` | S7.8 |
| `src/future_login.rs:354` | S7.7 |
| `src/future_login.rs:355` | S7.7 |
| `src/future_login.rs:482` | S7.7 |
| `src/desktop_shutdown/coordinator.rs:50` | S7.1 |
| `src/desktop_shutdown/coordinator.rs:165` | S7.12 |
| `src/desktop_shutdown/coordinator.rs:166` | S7.12 |
| `src/desktop_shutdown/coordinator.rs:183` | S7.12 |
| `src/desktop_shutdown/coordinator.rs:184` | S7.12 |
| `src/desktop_shutdown/coordinator.rs:185` | S7.12 |
| `src/desktop_shutdown/coordinator.rs:186` | S7.12 |
| `src/desktop_shutdown/coordinator.rs:187` | S7.12 |
| `src/macos_power.rs:21` | S7.2 |
| `src/macos_power.rs:22` | S7.2 |
| `src/macos_power.rs:23` | S7.2 |
| `src/macos_power.rs:24` | S7.2 |
| `src/macos_power.rs:26` | S7.2 |
| `src/macos_power.rs:27` | S7.2 |
| `src/macos_power.rs:29` | S7.2 |
| `src/macos_power.rs:30` | S7.2 |
| `src/macos_power.rs:31` | S7.2 |
| `src/macos_power.rs:32` | S7.2 |
| `src/runtime.rs:248` | S7.8 |
| `src/runtime.rs:249` | S7.8 |
| `src/runtime.rs:251` | S7.8 |
| `src/runtime.rs:252` | S7.8 |
| `src/instance.rs:164` | S7.6 |
| `src/instance.rs:165` | S7.6 |
| `src/instance.rs:166` | S7.6 |
| `src/instance.rs:167` | S7.6 |
| `src/instance.rs:168` | S7.6 |
| `src/bin/futureos-headless.rs:11` | S7.4 |

## S7.6 The todo's item (2): both remaining categories, decided with evidence

**(a) `desktop_shutdown.rs`:256-258 — still registered, and the blocker is a file I may not
write.** Measured in this run's report: 250–252 and 255 carry **25** hits each (the real
shutdown path runs), 256–258 are `DA:0`. As §S6.7.3 recorded, the arm needs a budget smaller
than `TOTAL_BUDGET = 20 s`, and the only entry point is
`fn begin_with_budget`, **private to `desktop_shutdown/coordinator.rs`**, which this todo's
declared write set does not include (`lib.rs`, `desktop_shutdown.rs`, this record). I did not
touch it. Two ways forward, both needing a decision that is not mine: add
`desktop_shutdown/coordinator.rs` to a successor's scope, or have its owner expose
`pub(super) fn begin_with_budget`. The state half is already available
(`remote::test_support::install_fake_state`).

**(b) `coordinator.rs`:165-166 and 183-187 — registered, no seam invented** (the two
`std::thread::Builder::spawn` error arms need an OS thread-creation refusal; the thread
names are constants, so there is no injection point).

## S7.7 The failures in this measurement, named (§5.3)

* **gui leg, 8 failures — all one mechanism, all peer-owned:** five
  `remote_peer::runtime_tests::{pairing_a_link_connects_and_reports_the_host,
  re_stating_the_same_conversation_keeps_the_lane_open,
  reconnecting_replaces_the_event_stream_rather_than_stacking_it,
  the_host_wide_mirror_is_read_only_while_the_lane_is_narrow}` and three
  `remote_peer::transfer_tests::{an_upload_round_trips_through_the_host,
  an_uploaded_file_can_be_claimed_as_an_attachment,
  forged_chunk_bytes_are_not_handed_to_a_waiting_pull,
  two_downloads_in_flight_pull_independently}`. Every one panics at
  `src/remote_peer/testing.rs:30` (`the test web port never freed`) after
  `web server bind on port 8022 failed: Address already in use (os error 48)` — the fixed
  `WEB_PORT` (`remote/diagnostics.rs:153`) held by other crate-test processes on this host
  (FLAKES.md's inventory). `src/remote_peer/**` is outside this unit's write set.
* **headless leg, 1 failure — a peer-owned integration test, and its own target passes in
  the other leg:** `tests/headless_cli.rs`'s
  `a_revoked_pairing_stops_the_run_with_the_reason_and_drops_the_credential` panicked at
  `tests/headless_cli.rs:521` with `the run did not exit within 300s` — a bounded wait that
  a loaded host (this leg took 609 s and peers were running) can exceed. The *same file* ran
  green in the gui leg of this same measurement (`tests/headless_cli.rs` → `2 passed`) and
  the headless leg's **lib** target is green (`1650 passed / 0 failed`). `tests/headless_cli.rs`
  is not in this unit's write set, and timeouts of that class are documented.
* **Direction argument:** none of the nine touches `gui::startup_paths_tests::*`; the arm
  this segment claims is verified per line in the union report (§S7.1), not inferred from an
  exit code; and the headless flavour — which does not even compile this segment's test
  (`mod gui` is `#[cfg(feature = "gui")]`) — is green in its lib target.

**Does the new fixture add failures? Measured, because that is the risk of a test that binds
port 8022.** The most port-sensitive peer test
(`remote::tests::runtime_tests::start_runs_the_full_bridge_and_stop_winds_it_down`) fails
**alone, twice** (baseline: `0 passed; 1 failed`, both runs) and with my fixture first it
failed once and passed once — i.e. that test's outcome is not determined by mine, and the
fixture does not make it worse. My fixture also waits (bounded) for 8022 to be bindable
before returning, which is what `remote::tests`' own bridge test does.

## S7.8 Tripwire and gates (PLAN §5.2 / §5.1)

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-c7union.info \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 7" --label tauri-shell-seg7
```

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **0** | clean crate-wide — no peer diff this time |
| format (this unit) | `rustfmt --edition 2021 --config skip_children=true --check src/lib.rs` | **0** | clean |
| clippy (production) | `cargo clippy --lib -- -D warnings` | **0** | clean |
| clippy (all targets) | `cargo clippy --all-targets -- -D warnings` | **0** | clean — the peer lints of Segment 6 have been fixed by their owners |
| this unit's fixtures (**17**, all of them) | `cargo test --lib -- --test-threads=1 <the 17 names>` | **0** | **17 passed / 0 failed** in 17.15 s, including the new one (1.8 s alone) |
| lib suite, gui (the measured leg) | §S7.1 | **101** | 2063 passed / 8 failed / 12 ignored — §S7.7 |
| lib suite, headless | §S7.1 | **0** | 1650 passed / 0 failed / 12 ignored |

## S7.9 Reproduction

```bash
 # 1. the measurement (§S7.1) and its completeness check
bash /tmp/tau-c7-measure.sh
grep -c "test store::threads" /tmp/tau-c7-measure.log      # must be > 0 (28 here)
 # 2. the new fixture, uninstrumented and fast (~2 s)
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --lib -- --test-threads=1 an_auto_connect_that_connects_is_reported_as_connected
 # 3. the port-8022 A/B of §S7.7
cargo test --lib -- --test-threads=1 remote::tests::runtime_tests::start_runs_the_full_bridge_and_stop_winds_it_down
 # 4. the tripwire and the gates (§S7.8)
```

# Segment 8 — the main-thread driver lands (route iii, a `harness = false` test target): the residual falls 238 → 150, and "needs the main thread" is retired from the ledger

Status: **measured, source changed** — a new test target
(`desktop/src-tauri/tests/gui_main_thread_drive.rs`, declared as
`[[test]] … harness = false, required-features = ["gui"]`) plus this record. Measured
revision: `lib.rs` sha256-prefix `8babf49b14d2d4e3`, the driver
`19b1404b81b5782a`, `Cargo.toml` `13bdc2a15ae6ebde`. Artifact of record:
**`/tmp/tauri-shell-lcov-c8union.info`** (`/tmp/tau-c8-measure.sh` → `/tmp/tau-c8-measure.log`,
18:39:23→18:58:57 UTC).

| scope residual | Segment 7 | **this segment** |
|---|---|---|
| total | 238 | **150** |

**The driver covered 88 lines** (measured, per line, §S8.2, against Segment 7's report of
the same files). §S5.3's **estimate was ≈87** — the measurement is within one line of it,
and the two clusters it named are exactly where the yield landed (`menu.rs` 48 → 2,
`lib.rs` 105 → 63).

## S8.1 What landed, and why as a test target

* **`tests/gui_main_thread_drive.rs`** — a `harness = false` target, so its `fn main()` *is*
  the process's main thread. It redirects `HOME`/`FUTURE_HOME` to a private temp root first
  (so the real `~/.future` is untouched, and `instance::gui_identifier` gives it a unique
  `/tmp/<identifier>_si.sock` because the data root is overridden), calls
  `futureos_lib::run_with(LaunchOptions::default())` — the same entry point `src/main.rs`
  uses — waits for the app's own `FutureOS: embedded Agent ready` line plus a 2 s settle
  window, then exits **through `std::process::exit`** (an `atexit`-running exit, which is
  what makes LLVM flush the profile — Appendix S7's fact 2). A hard 30 s deadline bounds it.
* **`Cargo.toml`** — the two-line stanza, with `required-features = ["gui"]` because
  `run_with` only exists in the gui build; the headless leg and CI's headless job therefore
  *skip* the target instead of failing to compile it (verified: `cargo clippy
  --no-default-features --features headless --all-targets -- -D warnings` → exit 0).
* **The interface requirement is met:** the driver's last output is exactly
  `gui_main_thread_drive: driver completed` (the harness accepts it as this target's summary
  in place of a `test result:` line), and the observed log order is
  `…embedded Agent ready` → the driver's diagnostic on stderr → the marker.
* **Cost and cleanliness, both measured:** the target runs in **≈4–5.4 s**
  (`time` on the raw binary: 4.1 s and 5.4 s across two runs; it is one extra target in the
  suite). Its temp root is removed on the way out — `find $TMPDIR -name 'futureos-gui-drive-*'`
  is **0** after a run (the first version leaked 928 KB per run; this crate has that scar).

## S8.2 Measured before/after, per file, line by line

`covlines.py`-equivalent per-file counts from the two union reports (Segment 7 = without the
driver, this segment = with it):

| file | Segment 7 | **this segment** | what the driver reached |
|---|---|---|---|
| `src/lib.rs` | 105 | **63** | −42 |
| `src/menu.rs` | 48 | **2** | −46 |
| `src/launch.rs` | 15 | 15 | — |
| `src/main.rs` | 21 | 21 | — |
| `src/desktop_shutdown.rs` | 18 | 18 | — |
| `src/future_login.rs` | 3 | 3 | — |
| `src/desktop_shutdown/coordinator.rs` | 8 | 8 | — |
| `src/macos_power.rs` | 10 | 10 | — |
| `src/runtime.rs` | 4 | 4 | — |
| `src/instance.rs` | 5 | 5 | — |
| `src/host_environment.rs` | 0 | 0 | — |
| `src/bin/futureos-headless.rs` | 1 | 1 | — |
| **total** | **238** | **150** | **−88** |

The two files that moved, in full (`/tmp/c8-lines.py` prints both directions; `LOST` was
empty in both, i.e. the driver added coverage and removed none):

* **`lib.rs`, 42 newly covered**: `209–215` (the window-geometry call's arguments — a real
  window and monitor), `474–478` (`set_macos_dock_icon`, a real AppKit call on the main
  thread), `811` (the streaming-monitor sample), `955–975` (the window-event hook's *outer*
  frame and the `CloseRequested` match), and the whole of `run_with`'s body and the `.setup`
  hook: `980, 984–986, 991–993, 995, 1005, 1008, 1010, 1011, 1031, 1032, 1034, 1038, 1039,
  1040, 1195, 1196, 1198, 1204, 1205, 1206`.
* **`menu.rs`, 46 of 48 newly covered**: `build_macos_menu` end to end (`26–40, 42–47,
  49, 51–59, 61–63, 65–70, 72–78`).

## S8.3 The category re-derivation (condition 2) — the important half

**Retired label:** `unreachable-in-this-environment` used to stand for this whole family on
the ground that "a libtest body is never the main thread". That ground is still true, and
**it is now irrelevant for everything the driver reaches**: those 88 lines are **covered**,
with no waiver at all, and the three groups that carried them in §S7.5 (S7.1's
`run_with`/setup/geometry part, S7.2's `menu.rs`, and the `RcBlock`-free part of
`macos_power`'s registration) are gone from the ledger.

What remains is classified by **what is actually missing** — each group is one concrete
input, not "a main thread":

| group | lines | what is actually missing |
|---|---|---|
| S8.1 | 12 | **a second instance**: the `tauri_plugin_single_instance` callback body (`906–924`). It runs when *another* process notifies this one; the driver is a single process |
| S8.2 | 11 | **a real menu click** (`932–950`, the `on_menu_event` body: About → `open-settings`, Restart Webview → reload) |
| S8.3 | 11 | **a real window close request** (`956–971`, the `on_window_event` body's quit-guard arms) |
| S8.4 | 11 | the **failure arms** of real calls: menu setup (`1004–1008`), window show/absent (`1033–1037`), terminal bind (`1227`), agent startup (`1252–1256`). On this host each call succeeds |
| S8.5 | 1 | **an Agent event reaching the real app's emitter** (`842`: needs `APP_HANDLE` set *and* a runtime update batched — the driver's process has the handle, but nothing emits into it) |
| S8.6 | 11 | `RunEvent::ExitRequested` (`1267–1279`) — `tauri::ExitRequestApi` has private fields and only Tauri's own event loop constructs it (§S6.7.2) |
| S8.7 | 3 | the **mobile entry point** `gui::run()` (`888–890`); no desktop caller exists (the bin calls `run_with`) |
| S8.8 | 2 | `menu.rs`'s two continuation lines (`41`, `48`) — the `)?)` of multi-line builder calls: region attribution to a continuation, not unexecuted behaviour (neighbours covered, function runs) |
| S8.9 | 7 | **test-internal invariants**: `lib.rs` 1727 (the burst producer's early `return`), 1906–1907 (`Failed::flush`'s `Ok`), `runtime.rs` 248–252 (the frozen registry's closure bodies — the test asserts they never run) |
| S8.10 | 36 | the **launcher process** and the **bin's own arms**: `launch.rs` 330–347 (spawn a real GUI Desktop and wait), `main.rs` 9, 29, 33–34, 38–51, 58, 70–75 (argv shapes, the re-pair hand-off, the helper dispatch). The driver calls `run_with` directly, so `main()`'s parsing never runs |
| S8.11 | 11 | `cfg(all(feature = "gui", not(test)))` blocks (`desktop_shutdown.rs` 221–223, 267–274) — **`unreachable-by-construction`**: the test cfg removes them |
| S8.12 | 3 | the **shutdown deadline arm** (`desktop_shutdown.rs` 256–258) — the budget knob is private to a file outside this scope (§S7.6) |
| S8.13 | 2 | the `freeze_tasks` race pair (`desktop_shutdown.rs` 499–500), §S2.8.4 |
| S8.14 | 1 | the `std::future::pending` sentinel (`desktop_shutdown.rs:501`) |
| S8.15 | 1 | a block-end brace llvm-cov attributes (`desktop_shutdown.rs:460`) — `attribution-artifact` |
| S8.16 | 3 | client-stack arms (`future_login.rs` 354–355, 482) — §6.5's measured result |
| S8.17 | 1 | `coordinator.rs:50` — the `APP_HANDLE`-needing arm |
| S8.18 | 7 | an **OS thread-creation refusal** (`coordinator.rs` 165–166, 183–187) |
| S8.19 | 10 | **real sleep/wake/power-off notifications** (`macos_power.rs`'s three observer bodies — the driver *registers* them, `lib.rs:995` is covered, but only the OS can fire them) |
| S8.20 | 5 | the **`flock` re-acquire window** (`instance.rs` 164–168) |
| S8.21 | 1 | the **Linux-only** helper success arm (`bin/futureos-headless.rs:11`) — `platform-unmeasured` |
| **total** | **150** | |

**Arithmetic:** `unreachable-in-this-environment` = **127** (S8.1 12 + S8.2 11 + S8.3 11 +
S8.4 11 + S8.5 1 + S8.6 11 + S8.7 3 + S8.10 36 + S8.12 3 + S8.13 2 + S8.16 3 + S8.17 1 +
S8.18 7 + S8.19 10 + S8.20 5) — every one of them naming a concrete input: a second
instance, a menu click, a window close request, a failing real call, an Agent event, a
private Tauri API, a caller that does not exist on desktop, a launcher process, a shutdown
budget, an OS thread refusal, a real power notification, or the `flock` re-acquire window.
`unreachable-by-construction` = **19** (S8.9 7 + S8.11 11 + S8.14 1),
`attribution-artifact` = **3** (S8.8 2 + S8.15 1), `platform-unmeasured` = **1** (S8.21).
`127 + 19 + 3 + 1 = 150` ✓ — and the per-line index below has exactly 150 rows.

**Flavour/platform sensitivity** (the supervisor asked): the driver needs the `gui` feature
and it is **macOS-relevant** — `menu.rs` and `macos_power.rs` are macOS-only, so on Linux a
run of this target reaches `run_with`/setup/geometry but not those two files. The frozen pass
should run it in the **gui** flavour on macOS; on Linux it is a clean skip-or-partial
(`run_with` returning `Err` on a display-less box prints a diagnostic and exits 0).

Checked mechanically before hand-off (PLAN §5.2) — §S8.5 has the invocation and its exit
code.

### Per-line index of this segment's residual (150 rows)

| line | group |
|---|---|
| `src/lib.rs:842` | S8.5 |
| `src/lib.rs:888` | S8.7 |
| `src/lib.rs:889` | S8.7 |
| `src/lib.rs:890` | S8.7 |
| `src/lib.rs:906` | S8.1 |
| `src/lib.rs:907` | S8.1 |
| `src/lib.rs:908` | S8.1 |
| `src/lib.rs:909` | S8.1 |
| `src/lib.rs:910` | S8.1 |
| `src/lib.rs:916` | S8.1 |
| `src/lib.rs:917` | S8.1 |
| `src/lib.rs:919` | S8.1 |
| `src/lib.rs:920` | S8.1 |
| `src/lib.rs:921` | S8.1 |
| `src/lib.rs:923` | S8.1 |
| `src/lib.rs:924` | S8.1 |
| `src/lib.rs:932` | S8.2 |
| `src/lib.rs:933` | S8.2 |
| `src/lib.rs:934` | S8.2 |
| `src/lib.rs:935` | S8.2 |
| `src/lib.rs:936` | S8.2 |
| `src/lib.rs:937` | S8.2 |
| `src/lib.rs:941` | S8.2 |
| `src/lib.rs:942` | S8.2 |
| `src/lib.rs:943` | S8.2 |
| `src/lib.rs:945` | S8.2 |
| `src/lib.rs:950` | S8.2 |
| `src/lib.rs:956` | S8.3 |
| `src/lib.rs:960` | S8.3 |
| `src/lib.rs:962` | S8.3 |
| `src/lib.rs:963` | S8.3 |
| `src/lib.rs:964` | S8.3 |
| `src/lib.rs:965` | S8.3 |
| `src/lib.rs:967` | S8.3 |
| `src/lib.rs:968` | S8.3 |
| `src/lib.rs:969` | S8.3 |
| `src/lib.rs:970` | S8.3 |
| `src/lib.rs:971` | S8.3 |
| `src/lib.rs:1004` | S8.4 |
| `src/lib.rs:1006` | S8.4 |
| `src/lib.rs:1007` | S8.4 |
| `src/lib.rs:1033` | S8.4 |
| `src/lib.rs:1035` | S8.4 |
| `src/lib.rs:1036` | S8.4 |
| `src/lib.rs:1037` | S8.4 |
| `src/lib.rs:1227` | S8.4 |
| `src/lib.rs:1252` | S8.4 |
| `src/lib.rs:1253` | S8.4 |
| `src/lib.rs:1256` | S8.4 |
| `src/lib.rs:1267` | S8.6 |
| `src/lib.rs:1268` | S8.6 |
| `src/lib.rs:1270` | S8.6 |
| `src/lib.rs:1271` | S8.6 |
| `src/lib.rs:1272` | S8.6 |
| `src/lib.rs:1273` | S8.6 |
| `src/lib.rs:1275` | S8.6 |
| `src/lib.rs:1276` | S8.6 |
| `src/lib.rs:1277` | S8.6 |
| `src/lib.rs:1278` | S8.6 |
| `src/lib.rs:1279` | S8.6 |
| `src/lib.rs:1727` | S8.9 |
| `src/lib.rs:1906` | S8.9 |
| `src/lib.rs:1907` | S8.9 |
| `src/menu.rs:41` | S8.8 |
| `src/menu.rs:48` | S8.8 |
| `src/launch.rs:330` | S8.10 |
| `src/launch.rs:331` | S8.10 |
| `src/launch.rs:332` | S8.10 |
| `src/launch.rs:333` | S8.10 |
| `src/launch.rs:334` | S8.10 |
| `src/launch.rs:338` | S8.10 |
| `src/launch.rs:339` | S8.10 |
| `src/launch.rs:340` | S8.10 |
| `src/launch.rs:341` | S8.10 |
| `src/launch.rs:342` | S8.10 |
| `src/launch.rs:343` | S8.10 |
| `src/launch.rs:344` | S8.10 |
| `src/launch.rs:345` | S8.10 |
| `src/launch.rs:346` | S8.10 |
| `src/launch.rs:347` | S8.10 |
| `src/main.rs:9` | S8.10 |
| `src/main.rs:29` | S8.10 |
| `src/main.rs:33` | S8.10 |
| `src/main.rs:34` | S8.10 |
| `src/main.rs:38` | S8.10 |
| `src/main.rs:39` | S8.10 |
| `src/main.rs:41` | S8.10 |
| `src/main.rs:42` | S8.10 |
| `src/main.rs:43` | S8.10 |
| `src/main.rs:44` | S8.10 |
| `src/main.rs:46` | S8.10 |
| `src/main.rs:47` | S8.10 |
| `src/main.rs:48` | S8.10 |
| `src/main.rs:51` | S8.10 |
| `src/main.rs:58` | S8.10 |
| `src/main.rs:70` | S8.10 |
| `src/main.rs:71` | S8.10 |
| `src/main.rs:72` | S8.10 |
| `src/main.rs:73` | S8.10 |
| `src/main.rs:74` | S8.10 |
| `src/main.rs:75` | S8.10 |
| `src/desktop_shutdown.rs:221` | S8.11 |
| `src/desktop_shutdown.rs:222` | S8.11 |
| `src/desktop_shutdown.rs:223` | S8.11 |
| `src/desktop_shutdown.rs:256` | S8.12 |
| `src/desktop_shutdown.rs:257` | S8.12 |
| `src/desktop_shutdown.rs:258` | S8.12 |
| `src/desktop_shutdown.rs:267` | S8.11 |
| `src/desktop_shutdown.rs:268` | S8.11 |
| `src/desktop_shutdown.rs:269` | S8.11 |
| `src/desktop_shutdown.rs:270` | S8.11 |
| `src/desktop_shutdown.rs:271` | S8.11 |
| `src/desktop_shutdown.rs:272` | S8.11 |
| `src/desktop_shutdown.rs:273` | S8.11 |
| `src/desktop_shutdown.rs:274` | S8.11 |
| `src/desktop_shutdown.rs:460` | S8.15 |
| `src/desktop_shutdown.rs:499` | S8.13 |
| `src/desktop_shutdown.rs:500` | S8.13 |
| `src/desktop_shutdown.rs:501` | S8.14 |
| `src/future_login.rs:354` | S8.16 |
| `src/future_login.rs:355` | S8.16 |
| `src/future_login.rs:482` | S8.16 |
| `src/desktop_shutdown/coordinator.rs:50` | S8.17 |
| `src/desktop_shutdown/coordinator.rs:165` | S8.18 |
| `src/desktop_shutdown/coordinator.rs:166` | S8.18 |
| `src/desktop_shutdown/coordinator.rs:183` | S8.18 |
| `src/desktop_shutdown/coordinator.rs:184` | S8.18 |
| `src/desktop_shutdown/coordinator.rs:185` | S8.18 |
| `src/desktop_shutdown/coordinator.rs:186` | S8.18 |
| `src/desktop_shutdown/coordinator.rs:187` | S8.18 |
| `src/macos_power.rs:21` | S8.19 |
| `src/macos_power.rs:22` | S8.19 |
| `src/macos_power.rs:23` | S8.19 |
| `src/macos_power.rs:24` | S8.19 |
| `src/macos_power.rs:26` | S8.19 |
| `src/macos_power.rs:27` | S8.19 |
| `src/macos_power.rs:29` | S8.19 |
| `src/macos_power.rs:30` | S8.19 |
| `src/macos_power.rs:31` | S8.19 |
| `src/macos_power.rs:32` | S8.19 |
| `src/runtime.rs:248` | S8.9 |
| `src/runtime.rs:249` | S8.9 |
| `src/runtime.rs:251` | S8.9 |
| `src/runtime.rs:252` | S8.9 |
| `src/instance.rs:164` | S8.20 |
| `src/instance.rs:165` | S8.20 |
| `src/instance.rs:166` | S8.20 |
| `src/instance.rs:167` | S8.20 |
| `src/instance.rs:168` | S8.20 |
| `src/bin/futureos-headless.rs:11` | S8.21 |

## S8.4 The gates (PLAN §5.1) and the suites

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **0** | clean crate-wide. (First run was **1** with one diff — **this segment's new file** at `tests/gui_main_thread_drive.rs:101`; `rustfmt` applied and re-checked **0**. Recorded because it is the gate catching my own code, which is the point of running it) |
| clippy (all targets) | `cargo clippy --all-targets -- -D warnings` | **0** | clean, and it covers the new test target |
| clippy (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **0** | clean — the new target is *skipped* by `required-features = ["gui"]`, not broken |
| the driver, uninstrumented | `cargo test --test gui_main_thread_drive` | **0** | prints the marker as its last line; **≈4–5.4 s** |
| lib suite, gui (the measured leg) | §S8.1 | **101** | lib `2069 passed / 3 failed / 12 ignored` (547.56 s) — the failures are the port-8022 `remote_peer` family, named in §S8.5 |
| lib suite, headless (the measured leg) | §S8.1 | **0** | lib `1651 passed / 0 failed / 12 ignored` (green) |
| completeness check on the gui leg (condition 3) | `grep -c "test store::threads" /tmp/tau-c8-measure.log` | — | **58** (tail modules ran) and the lib target printed its `test result` line; the driver's own marker appears in the same leg (`gui_main_thread_drive: driver completed`) |

## S8.5 Tripwire (PLAN §5.2) and the failures, named

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-c8union.info \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 8" --label tauri-shell-seg8
```

**The three gui failures, all peer-owned and all one mechanism:** three
`remote_peer::*` tests panicking at `src/remote_peer/testing.rs:30` (`the test web port never
freed`) after `web server bind on port 8022 failed: Address already in use` — the fixed
`WEB_PORT` held by other crate-test processes on this host (FLAKES.md's inventory); the set
varies run to run (Segment 7 saw eight of the same family, Segment 6 one). `src/remote_peer/**`
is outside this unit's write set, and the headless leg — which does not compile this
segment's target — is green.

## S8.6 Reproduction

```bash
 # 1. the driver alone (≈5 s), and its marker
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --test gui_main_thread_drive            # last line: gui_main_thread_drive: driver completed
 # 2. the measurement (§S8.1) and its completeness check (condition 3)
bash /tmp/tau-c8-measure.sh
grep -c "test store::threads" /tmp/tau-c8-measure.log      # must be > 0 (58 here)
grep -c "gui_main_thread_drive: driver completed" /tmp/tau-c8-measure.log
 # 3. the per-line yield and the category evidence
python3 /tmp/c8-diff.py ; python3 /tmp/c8-lines.py
 # 4. the tripwire (§S8.5) and the gates (§S8.4)
```

# Segment 9 — the single-instance callback is driven from a second process (9 of its 12 lines covered), and this segment's own measurement records a NET REGRESSION it must own

Status: **measured, source changed, partial** — `tests/gui_main_thread_drive.rs` gained the
notification step. Artifact of record: **`/tmp/tauri-shell-lcov-c9union.info`**
(`/tmp/tau-c9-measure.sh` → `/tmp/tau-c9-measure.log`, 19:21:48→19:48:33 UTC).

| scope residual | Segment 8 | **this segment (measured)** |
|---|---|---|
| total | **150** | **220** |

**Read this before the numbers: the target was achieved and the total went the wrong way.**
Nine of the twelve `S8.1` lines are now covered — the `tauri_plugin_single_instance` callback
body really executes, driven from a second process — but this run **lost 70 lines that
Segment 8's artifact had covered** (`menu.rs` 46 of them), because the driver finished before
the app reached `RunEvent::Ready` on this host. §S9.3 separates the two causes and §S9.4
records the fix, which is in the file but **not yet measured**. The S8 artifact stays the
pinned one (`/tmp/tauri-shell-lcov-c8union.info`, 150) until a re-measure on a calm host
confirms or refutes the recovery; nothing here claims 12/12 or a new low.

## S9.1 What landed

* **The driver now drives the plugin's callback.** After the app is up it spawns **two
  instances of itself** with the plugin's own two notification shapes —
  `--re-pair` (`reply: None`, via `launch::notifier_command`-equivalent `argv`) and the
  launcher's hand-off `--re-pair --re-pair-reply <path>` (`reply: Some(path)`). The children
  inherit `FUTUREOS_DRIVE_ROOT`, so they land on **this** process's socket
  (`/tmp/com_futureos_desktop.home<root_id[:16]>_si.sock`, computed by the driver the way the
  plugin computes it, `future_rpc::home`/`connection::data_root_id`), connect, write
  `cwd\0\0argv`, and exit 0 inside the plugin's `setup` (§S6.7.1's mechanism). The parent's
  listener then delivers the message to the app's callback.
* **The reply file is the attribution.** The callback's `Reply(path)` arm spawns the re-pair
  pass, which answers on that file even when it fails; the driver waits (bounded) for it and
  reports the answer. Observed in the instrumented leg:
  `the forwarded re-pair was answered on …json: { "error": "the remote start ended in Failed
  (AccountAuthorization)" }` — and the sibling `Terminal` delivery printed
  `The Remote page in the Desktop window shows the current state.` (the callback's other arm).
* **A companion flag prevents self-recursion.** A `harness = false` target that spawns *itself*
  recurses: my first version produced children that each spawned grandchildren (measured: 5+
  nested "child exceeded 20s" messages, 37 s, exponential). Children are identified by
  `FUTUREOS_DRIVE_ROOT` being set and never notify.
* **The isolated root must exist before the socket can be named** (`data_root_id` canonicalises
  it) and only the primary removes it (children share it).

## S9.2 The target lines, measured

From `/tmp/tauri-shell-lcov-c9union.info`, against Segment 8's artifact:

| line | c8 | **c9** |
|---|---|---|
| `906` (`if let Some(window) = app.get_webview_window("main")`) | 0 | **8** |
| `907`–`909` (unminimize / show / set_focus) | 0 | **0** |
| `910` (the block's brace) | 0 | **8** |
| `916`–`917` (the `Forwarded::RePair` match) | 0 | **2 / 8** |
| `919`–`921` (the `reply` match, both arms) | 0 | **2 / 1 / 1** |
| `923`–`924` (the callback's tail) | 0 | **6 / 8** |

**9 of 12 covered.** `907`–`909` are the three window calls: the callback ran, but
`get_webview_window("main")` returned `None` because the notification arrived before `Ready`
creates the window. The readiness-gated variant (§S9.4) closes exactly that gap.

## S9.3 The 70 lost lines, and the two causes kept apart

| cause | evidence | whose |
|---|---|---|
| **this segment's timing**: the driver left before the app reached `Ready`, so the setup hook never ran — `menu.rs` 48 → 48 (all of `build_macos_menu`), `lib.rs` 993/995/1005/1008/1010/1011 (the hook), 209–215/474–478 (the geometry and dock-icon calls it makes), 955–975/1031–1039 (the window-event frame and window show), 811, `runtime.rs` 4 → 8 | the leg's log has the driver's completion message and **no** `embedded Agent ready`; the same 10 lines were covered in the S8 artifact, where it did appear | **mine — fixed in the file, not yet measured** |
| **the host/tree's readiness latency**: `Ready` took ~2.5 s when Segment 8 measured, 30–40 s under load an hour later, and did **not** appear within 90 s in this segment's last verification run (load 3.35) | `/tmp/c9-patient.log` (readiness at ~35 s), `/tmp/c9-run11.log`, and the c9 leg itself | **not mine to fix**: it is the embedded Agent's own startup in a tree several units are rebuilding; it must be reported, not hidden, because it decides whether *any* main-thread measurement here is reproducible today |

So the honest reading of `150 → 220` is: **+9 for the work, −70 from a driver that now leaves
too early, of which 46 are the menu family that the same driver covered one segment ago.** A
number that moves the wrong way for a *fixture-timing* reason is still a regression in the
metric, and it is recorded as one.

## S9.4 The fix, in the file and not yet measured

The watchdog now has **one trigger**: it waits (bounded by `READY_BUDGET = 90 s`) for the app's
own `embedded Agent ready` line, and only then runs the notification step, with `SETTLE` after
it. That (a) preserves the setup-hook coverage `Ready` brings, (b) makes the callback's window
lookups succeed, so 907–909 are covered too, and (c) still bounds the target's cost
(idle ≈ 5–7 s; the budget is the ceiling, and every phase — listener, children, reply, settle —
is bounded as well).

Two intermediate designs were built and measured, and both are recorded because they are the
kind of thing that looks right on paper:
* **readiness-gated with a 30 s budget** (Segment 8's shape): too tight under load — the app
  needs 30–40 s, so the driver left before the setup hook and lost the family above;
* **listener-gated** (notify ~2 s in, independent of `Ready`): load-insensitive and it did
  cover 906/910/916–924, but it exits before `Ready` by design, which *is* the 70-line loss,
  and it skips 907–909. Rejected.

The corrected file has already been verified to run end-to-end (children exit 0, reply file
answered, marker last, isolated root removed, no leaked dirs) but **not** to reach `Ready` on
this host, so its yield is unmeasured. That is the successor's first item.

## S9.5 The residual (220 lines) — groups

**Superseded by §S10.1/§S10.3:** the readiness gate this section's fix relies on cannot be
satisfied in the session that has been live since 19:11:53 UTC, so the driver now notifies
unconditionally and treats the readiness wait as a tight, overridable bound. The groups below
are kept because §S10.4 re-derives from them, and the group ids in §S10.4's index are the
same mechanisms with `S10.` prefixes.

| group | lines | what is missing |
|---|---|---|
| S9.1 | 3 | the callback's three window calls (`907`–`909`): covered once the notification follows `Ready` (§S9.4) — **not waived** |
| S9.2 | 29 | the setup-hook/`Ready`-window family this run lost (`209`–`215`, `474`–`478`, `811`, `955`–`975`, `993`–`1039` minus the covered ones): covered by the same driver on an unloaded host (§S9.3) — **not waived** |
| S9.3 | 11 | a real **menu click** (`on_menu_event` body) |
| S9.4 | 11 | a real **window close request** (`on_window_event` body) |
| S9.5 | 11 | the **failure arms** of real calls that all succeed on this host (menu setup, window show/absent, terminal bind, agent startup) |
| S9.6 | 1 | an **Agent event reaching the real emitter** (`842`) |
| S9.7 | 11 | `RunEvent::ExitRequested` — `tauri::ExitRequestApi` has private fields |
| S9.8 | 3 | the **mobile entry point** (`888`–`890`), no desktop caller |
| S9.9 | 36 | the **launcher process** and the bin's `argv` arms (`launch.rs`, `main.rs`) |
| S9.10 | 12 | test-internal invariants (`1727`, `1906`–`1907`, `desktop_shutdown:501`, `runtime.rs`'s frozen-registry closures) — `unreachable-by-construction` |
| S9.11 | 48 | `menu.rs` (`build_macos_menu`) — **covered on an unloaded host**, see S9.2 |
| S9.12 | 11 | `cfg(all(feature = "gui", not(test)))` blocks — `unreachable-by-construction` |
| S9.13 | 3 | the shutdown deadline arm (budget knob is in a file outside this scope) |
| S9.15 | 2 | the `freeze_tasks` race pair |
| S9.16 | 1 | a block-end brace — `attribution-artifact` |
| S9.17 | 3 | client-stack arms (`future_login.rs`) |
| S9.18/19 | 1 + 7 | the `APP_HANDLE` arm and the OS thread-refusal arms (`coordinator.rs`) |
| S9.20 | 10 | real **sleep/wake/power-off notifications** (`macos_power.rs`) |
| S9.21 | 5 | the `flock` re-acquire window |
| S9.22 | 1 | the Linux-only helper arm — `platform-unmeasured` |
| **total** | **220** | |

`S9.1 + S9.2 + S9.11 = 80` of the 220 are the driver's own reach (30 + 48 + 3, of which S9.1
and S9.2 are this run's artifacts and S9.11 is the family it lost): **they are not waivers**,
and the next measurement is expected to remove them. The remaining 140 are the same
mechanisms Segment 8 catalogued (real OS events, the launcher process, private Tauri APIs,
test-only invariants, cfg-excluded code, the peer-owned budget knob).

### Per-line index of this segment's residual (220 rows)

| line | group |
|---|---|
| `src/lib.rs:209` | S9.2 |
| `src/lib.rs:210` | S9.2 |
| `src/lib.rs:211` | S9.2 |
| `src/lib.rs:212` | S9.2 |
| `src/lib.rs:213` | S9.2 |
| `src/lib.rs:214` | S9.2 |
| `src/lib.rs:215` | S9.2 |
| `src/lib.rs:474` | S9.2 |
| `src/lib.rs:475` | S9.2 |
| `src/lib.rs:476` | S9.2 |
| `src/lib.rs:477` | S9.2 |
| `src/lib.rs:478` | S9.2 |
| `src/lib.rs:811` | S9.2 |
| `src/lib.rs:842` | S9.6 |
| `src/lib.rs:888` | S9.8 |
| `src/lib.rs:889` | S9.8 |
| `src/lib.rs:890` | S9.8 |
| `src/lib.rs:907` | S9.1 |
| `src/lib.rs:908` | S9.1 |
| `src/lib.rs:909` | S9.1 |
| `src/lib.rs:932` | S9.3 |
| `src/lib.rs:933` | S9.3 |
| `src/lib.rs:934` | S9.3 |
| `src/lib.rs:935` | S9.3 |
| `src/lib.rs:936` | S9.3 |
| `src/lib.rs:937` | S9.3 |
| `src/lib.rs:941` | S9.3 |
| `src/lib.rs:942` | S9.3 |
| `src/lib.rs:943` | S9.3 |
| `src/lib.rs:945` | S9.3 |
| `src/lib.rs:950` | S9.3 |
| `src/lib.rs:955` | S9.2 |
| `src/lib.rs:956` | S9.4 |
| `src/lib.rs:957` | S9.2 |
| `src/lib.rs:958` | S9.2 |
| `src/lib.rs:960` | S9.4 |
| `src/lib.rs:962` | S9.4 |
| `src/lib.rs:963` | S9.4 |
| `src/lib.rs:964` | S9.4 |
| `src/lib.rs:965` | S9.4 |
| `src/lib.rs:967` | S9.4 |
| `src/lib.rs:968` | S9.4 |
| `src/lib.rs:969` | S9.4 |
| `src/lib.rs:970` | S9.4 |
| `src/lib.rs:971` | S9.4 |
| `src/lib.rs:974` | S9.2 |
| `src/lib.rs:975` | S9.2 |
| `src/lib.rs:993` | S9.2 |
| `src/lib.rs:995` | S9.2 |
| `src/lib.rs:1004` | S9.5 |
| `src/lib.rs:1005` | S9.2 |
| `src/lib.rs:1006` | S9.5 |
| `src/lib.rs:1007` | S9.5 |
| `src/lib.rs:1008` | S9.2 |
| `src/lib.rs:1010` | S9.2 |
| `src/lib.rs:1011` | S9.2 |
| `src/lib.rs:1031` | S9.2 |
| `src/lib.rs:1032` | S9.2 |
| `src/lib.rs:1033` | S9.5 |
| `src/lib.rs:1034` | S9.2 |
| `src/lib.rs:1035` | S9.5 |
| `src/lib.rs:1036` | S9.5 |
| `src/lib.rs:1037` | S9.5 |
| `src/lib.rs:1038` | S9.2 |
| `src/lib.rs:1039` | S9.2 |
| `src/lib.rs:1227` | S9.5 |
| `src/lib.rs:1252` | S9.5 |
| `src/lib.rs:1253` | S9.5 |
| `src/lib.rs:1256` | S9.5 |
| `src/lib.rs:1267` | S9.7 |
| `src/lib.rs:1268` | S9.7 |
| `src/lib.rs:1270` | S9.7 |
| `src/lib.rs:1271` | S9.7 |
| `src/lib.rs:1272` | S9.7 |
| `src/lib.rs:1273` | S9.7 |
| `src/lib.rs:1275` | S9.7 |
| `src/lib.rs:1276` | S9.7 |
| `src/lib.rs:1277` | S9.7 |
| `src/lib.rs:1278` | S9.7 |
| `src/lib.rs:1279` | S9.7 |
| `src/lib.rs:1727` | S9.10 |
| `src/lib.rs:1906` | S9.10 |
| `src/lib.rs:1907` | S9.10 |
| `src/menu.rs:26` | S9.11 |
| `src/menu.rs:27` | S9.11 |
| `src/menu.rs:28` | S9.11 |
| `src/menu.rs:29` | S9.11 |
| `src/menu.rs:31` | S9.11 |
| `src/menu.rs:32` | S9.11 |
| `src/menu.rs:33` | S9.11 |
| `src/menu.rs:34` | S9.11 |
| `src/menu.rs:35` | S9.11 |
| `src/menu.rs:36` | S9.11 |
| `src/menu.rs:37` | S9.11 |
| `src/menu.rs:38` | S9.11 |
| `src/menu.rs:39` | S9.11 |
| `src/menu.rs:40` | S9.11 |
| `src/menu.rs:41` | S9.11 |
| `src/menu.rs:42` | S9.11 |
| `src/menu.rs:43` | S9.11 |
| `src/menu.rs:44` | S9.11 |
| `src/menu.rs:45` | S9.11 |
| `src/menu.rs:46` | S9.11 |
| `src/menu.rs:47` | S9.11 |
| `src/menu.rs:48` | S9.11 |
| `src/menu.rs:49` | S9.11 |
| `src/menu.rs:51` | S9.11 |
| `src/menu.rs:52` | S9.11 |
| `src/menu.rs:53` | S9.11 |
| `src/menu.rs:54` | S9.11 |
| `src/menu.rs:55` | S9.11 |
| `src/menu.rs:56` | S9.11 |
| `src/menu.rs:57` | S9.11 |
| `src/menu.rs:58` | S9.11 |
| `src/menu.rs:59` | S9.11 |
| `src/menu.rs:61` | S9.11 |
| `src/menu.rs:62` | S9.11 |
| `src/menu.rs:63` | S9.11 |
| `src/menu.rs:65` | S9.11 |
| `src/menu.rs:66` | S9.11 |
| `src/menu.rs:67` | S9.11 |
| `src/menu.rs:68` | S9.11 |
| `src/menu.rs:69` | S9.11 |
| `src/menu.rs:70` | S9.11 |
| `src/menu.rs:72` | S9.11 |
| `src/menu.rs:73` | S9.11 |
| `src/menu.rs:74` | S9.11 |
| `src/menu.rs:75` | S9.11 |
| `src/menu.rs:76` | S9.11 |
| `src/menu.rs:77` | S9.11 |
| `src/menu.rs:78` | S9.11 |
| `src/launch.rs:330` | S9.9 |
| `src/launch.rs:331` | S9.9 |
| `src/launch.rs:332` | S9.9 |
| `src/launch.rs:333` | S9.9 |
| `src/launch.rs:334` | S9.9 |
| `src/launch.rs:338` | S9.9 |
| `src/launch.rs:339` | S9.9 |
| `src/launch.rs:340` | S9.9 |
| `src/launch.rs:341` | S9.9 |
| `src/launch.rs:342` | S9.9 |
| `src/launch.rs:343` | S9.9 |
| `src/launch.rs:344` | S9.9 |
| `src/launch.rs:345` | S9.9 |
| `src/launch.rs:346` | S9.9 |
| `src/launch.rs:347` | S9.9 |
| `src/main.rs:9` | S9.9 |
| `src/main.rs:29` | S9.9 |
| `src/main.rs:33` | S9.9 |
| `src/main.rs:34` | S9.9 |
| `src/main.rs:38` | S9.9 |
| `src/main.rs:39` | S9.9 |
| `src/main.rs:41` | S9.9 |
| `src/main.rs:42` | S9.9 |
| `src/main.rs:43` | S9.9 |
| `src/main.rs:44` | S9.9 |
| `src/main.rs:46` | S9.9 |
| `src/main.rs:47` | S9.9 |
| `src/main.rs:48` | S9.9 |
| `src/main.rs:51` | S9.9 |
| `src/main.rs:58` | S9.9 |
| `src/main.rs:70` | S9.9 |
| `src/main.rs:71` | S9.9 |
| `src/main.rs:72` | S9.9 |
| `src/main.rs:73` | S9.9 |
| `src/main.rs:74` | S9.9 |
| `src/main.rs:75` | S9.9 |
| `src/desktop_shutdown.rs:221` | S9.12 |
| `src/desktop_shutdown.rs:222` | S9.12 |
| `src/desktop_shutdown.rs:223` | S9.12 |
| `src/desktop_shutdown.rs:256` | S9.13 |
| `src/desktop_shutdown.rs:257` | S9.13 |
| `src/desktop_shutdown.rs:258` | S9.13 |
| `src/desktop_shutdown.rs:267` | S9.12 |
| `src/desktop_shutdown.rs:268` | S9.12 |
| `src/desktop_shutdown.rs:269` | S9.12 |
| `src/desktop_shutdown.rs:270` | S9.12 |
| `src/desktop_shutdown.rs:271` | S9.12 |
| `src/desktop_shutdown.rs:272` | S9.12 |
| `src/desktop_shutdown.rs:273` | S9.12 |
| `src/desktop_shutdown.rs:274` | S9.12 |
| `src/desktop_shutdown.rs:460` | S9.16 |
| `src/desktop_shutdown.rs:499` | S9.15 |
| `src/desktop_shutdown.rs:500` | S9.15 |
| `src/desktop_shutdown.rs:501` | S9.10 |
| `src/future_login.rs:354` | S9.17 |
| `src/future_login.rs:355` | S9.17 |
| `src/future_login.rs:482` | S9.17 |
| `src/desktop_shutdown/coordinator.rs:50` | S9.18 |
| `src/desktop_shutdown/coordinator.rs:165` | S9.19 |
| `src/desktop_shutdown/coordinator.rs:166` | S9.19 |
| `src/desktop_shutdown/coordinator.rs:183` | S9.19 |
| `src/desktop_shutdown/coordinator.rs:184` | S9.19 |
| `src/desktop_shutdown/coordinator.rs:185` | S9.19 |
| `src/desktop_shutdown/coordinator.rs:186` | S9.19 |
| `src/desktop_shutdown/coordinator.rs:187` | S9.19 |
| `src/macos_power.rs:21` | S9.20 |
| `src/macos_power.rs:22` | S9.20 |
| `src/macos_power.rs:23` | S9.20 |
| `src/macos_power.rs:24` | S9.20 |
| `src/macos_power.rs:26` | S9.20 |
| `src/macos_power.rs:27` | S9.20 |
| `src/macos_power.rs:29` | S9.20 |
| `src/macos_power.rs:30` | S9.20 |
| `src/macos_power.rs:31` | S9.20 |
| `src/macos_power.rs:32` | S9.20 |
| `src/runtime.rs:46` | S9.10 |
| `src/runtime.rs:64` | S9.10 |
| `src/runtime.rs:65` | S9.10 |
| `src/runtime.rs:66` | S9.10 |
| `src/runtime.rs:248` | S9.10 |
| `src/runtime.rs:249` | S9.10 |
| `src/runtime.rs:251` | S9.10 |
| `src/runtime.rs:252` | S9.10 |
| `src/instance.rs:164` | S9.21 |
| `src/instance.rs:165` | S9.21 |
| `src/instance.rs:166` | S9.21 |
| `src/instance.rs:167` | S9.21 |
| `src/instance.rs:168` | S9.21 |
| `src/bin/futureos-headless.rs:11` | S9.22 |

## S9.6 Tripwire, gates, and what the record claims

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-c9union.info \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 9" --label tauri-shell-seg9
 #   → uncovered lines=220  mentioned=220  MISSING=0  (exit 0)
```

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **0** | clean crate-wide; the driver file is also clean on its own (`rustfmt --edition 2021 --config skip_children=true --check tests/gui_main_thread_drive.rs` → 0) |
| clippy (all targets) | `cargo clippy --all-targets -- -D warnings` | **0** | clean, and it covers the driver target |
| clippy (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **0** | clean — the driver is skipped by `required-features = ["gui"]` |
| the driver, uninstrumented | `cargo test --test gui_main_thread_drive` | **0** | runs end-to-end: two children exit 0, the reply file is answered, the marker is the last line, the isolated root is removed (0 leaked dirs) |
| lib suite, gui (the measured leg) | §S9.1 | **101** | `2072 passed / 2 failed / 12 ignored` in 556 s; both failures are the port-8022 `remote_peer` family |
| lib suite, headless (the measured leg) | §S9.1 | **101** | the lib target is green in that leg; the 101 came from a peer-owned `tests/headless_cli.rs` bounded wait (the same file passed in the gui leg) |
| the driver inside the instrumented leg | the leg's log | — | `driver completed … listener_up=true elapsed=22.6s`; both children exited 0 in 0.17 s; the reply file was answered |

**What this segment claims, exactly.** The single-instance callback body is reachable from a
test target and **9 of its 12 lines are covered in the measured artifact**; the scope total in
that same artifact is **220** — **70 worse** than Segment 8's pinned 150, of which 46 are the
`menu.rs` family the identical driver covered one segment earlier; the cause is the driver
leaving before the app reached `Ready` on this host, and the fix (readiness-gated
notification, `READY_BUDGET = 90 s`, §S9.4) is in the file but **unmeasured**. Segment 8's
artifact remains the pinned number for this unit.

## S9.7 Reproduction

```bash
 # 1. the driver alone (idle: ~5-7 s; loaded: up to the 90 s budget)
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --test gui_main_thread_drive          # last line: gui_main_thread_drive: driver completed
 # 2. the measurement (§S9.1) and its completeness checks
bash /tmp/tau-c9-measure.sh
grep -c "test store::threads" /tmp/tau-c9-measure.log
grep -E "^gui_main_thread_drive:" /tmp/tau-c9-measure.log
 # 3. the per-line target evidence (§S9.2) and the loss (§S9.3)
python3 /tmp/c9-diff.py
 # 4. the tripwire and the gates (§S9.6)
```

# Segment 10 — the driver no longer depends on the host delivering `Ready`; the residual reads 217, and `Ready` is measured ABSENT in this GUI session

Segment 9 left the driver gated on `embedded Agent ready`, a line the `Ready` arm's
supervision prints (§S9.4). This segment reworks that gate, re-measures, and records the
host-state finding that explains the S8→S9 swing: **the pinned Segment 8 artifact (150) was
taken in a GUI session that delivered `RunEvent::Ready`; the session that has been live since
19:11:53 UTC does not deliver it**, and no measurement taken in it can recover the lines that
depend on `Ready`.

## S10.1 What changed — test target only, no production file touched

| change | why |
|---|---|
| the single-instance notification is now sent **unconditionally** once the plugin's listener is up (it was gated on the readiness line) | the plugin delivers the callback on its own thread, so the nine callback lines do not need `Ready` at all — §S9.2 measured exactly that (the reply file was answered while the request never reached `Ready`). Gating on `Ready` made the target's own coverage hostage to a host state it cannot control |
| the readiness wait survives, **tight** — 15 s, was 90 s — and is overridable with `FUTUREOS_DRIVE_READY_BUDGET_SECS` | it is what earns the callback's window arms and preserves the `.setup` hook family *on a host that delivers `Ready`*; as a bound it must not cost minutes in every suite run |
| the reply wait is bounded at 6 s (was 10 s) and the listener wait at 30 s (was 60 s) | same reason: this target runs in every plain and instrumented invocation |
| the "completing without the notification step" path is gone, and the request is annotated in the walkthrough (`context.config_mut().identifier` stays as §S6.7.1 fixed it) | there is no longer a branch that skips the notification |

Measured cost: **19.5 s** in the host state of §S10.3 (`without` readiness — the readiness
budget plus the reply wait); ≈5–8 s when readiness is delivered. The marker is still the last
line, the isolated root is still removed (0 leaked), and the `Cargo.toml` stanza is unchanged
(`harness = false`, `required-features = ["gui"]`).

## S10.2 The measurement

```bash
bash /tmp/tau-c10-measure.sh
```

| step | exit | result |
|---|---|---|
| `make desktop-sidecar-placeholder` | 0 | |
| `cargo llvm-cov clean` | 0 | a full instrumented rebuild followed |
| gui leg, `cargo llvm-cov --no-report … -- --test-threads=1` | **0** | lib target `2076 passed; 0 failed; 12 ignored` in 524.61 s; the driver target ran; `tests/headless_cli.rs` ran |
| `cargo llvm-cov report --lcov` (gui) | 0 | `/tmp/tauri-shell-lcov-c10-gui.info` |
| headless leg (`--no-default-features --features headless`) | **101** | **truncated at the target level: its `--lib` target was killed by SIGTERM** (see §S11.7.1) — no `test result:` summary for it, `error: 1 target failed`. The other three targets completed (main 2/0, bin 1/0, `headless_cli` 12/0). Test-level failures in the target that did report: **4**, all `remote_peer` — `runtime_tests::connect_all_resumes_a_host_the_user_had_stopped`, `runtime_tests::connecting_an_unknown_host_is_an_error_not_an_invention`, `runtime_tests::connecting_replaces_an_existing_retry_loop`, `runtime_tests::disconnect_drops_the_connection_but_keeps_the_pairing` (the documented peer/host family, **outside this unit**). **Corrected in §S11.7.3:** this row first listed five names, two of them wrong. |
| `cargo llvm-cov report --lcov` (headless) | 0 | `/tmp/tauri-shell-lcov-c10-headless.info` |
| `python3 /tmp/lcov-union.py` | 0 | `/tmp/tauri-shell-lcov-c10union.info` |

Completeness checks (§S6.7.1's hazard). Per leg, `Running` lines against `test result:` summaries
(the full accounting is §S11.7.2):

* **gui leg — complete.** 4 targets, 3 summaries **plus the driver's marker** (a
  `harness = false` target prints no summary by design), **0** test failures, the lib summary
  present (`2076 passed; 0 failed; 12 ignored`, 524.61 s), and the tail modules ran
  (`grep -c "test store::threads"` → **30**).
* **headless leg — truncated.** 4 targets, **3** summaries: the `--lib` target was killed
  (§S11.7), so it printed none. This is exactly the state `measure-final.sh`'s
  `Running`-vs-summary guard exists to flag.

Per file — the twelve ledger scopes, each column rendered by `covlines.py total --report …
--under <scope>` from that segment's own artifact:

| scope | S8 (pinned) | S9 | **S10** |
|---|---|---|---|
| `src/lib.rs` | 63 | 83 | **83** |
| `src/menu.rs` | 2 | 48 | **48** |
| `src/main.rs` | 21 | 21 | **21** |
| `src/desktop_shutdown.rs` | 18 | 18 | **18** |
| `src/launch.rs` | 15 | 15 | **15** |
| `src/macos_power.rs` | 10 | 10 | **10** |
| `src/desktop_shutdown/` (coordinator.rs) | 8 | 8 | **8** |
| `src/instance.rs` | 5 | 5 | **5** |
| `src/runtime.rs` | 4 | 8 | **5** |
| `src/future_login.rs` | 3 | 3 | **3** |
| `src/bin/` | 1 | 1 | **1** |
| `src/host_environment.rs` | 0 | 0 | **0** |
| **total** | **150** | **220** | **217** |

Line-region detail: measured lines 3839 (S8 had 3687 — the driver's own tests and the peers'
landings enlarged the region map), and **zero** lines flipped from covered to uncovered
relative to Segment 9 (`S10 \ S9 = ∅`), so this segment added no `DA:0` of its own.

## S10.3 `RunEvent::Ready` is not delivered in this GUI session

The chain is pinned to source: `RunEvent::Ready` is emitted on tao's
`Event::NewEvents(StartCause::Init)` (`tauri-runtime-wry-2.11.4/src/lib.rs:4173-4174`), and
tao emits `Init` from `AppState::launched` (`tao-0.35.3/src/platform_impl/macos/app_state.rs:283-306`),
which is the body of the app delegate's `applicationDidFinishLaunching:`.

**Probe.** The `Ready` arm calls `terminal::server::serve()` (lib.rs:1261 region), while
`terminal::server::bind()` runs *before* the event loop (`initialize_process_state`). So a
bound listener that never accepts proves the arm did not run: 4/4 app runs showed the socket
in `LISTEN` with `curl` returning `http_code=000` after 4 s, the app's own log holding exactly
one line (`host starting`), the main thread idling in `-[NSApplication run]` →
`mach_msg2_trap`, and every tokio worker parked. A separate 94 s run behaved identically.

**Alternatives that were measured and excluded:**

| candidate | measurement |
|---|---|
| host load | the failures occur at load 5.9–8.3 (and the successes at 18:58 were taken at a *higher* load) |
| file-descriptor limit | identical at `ulimit -n 2560` and `10240` (2 trials each) |
| this unit's notify children | companion mode (which never notifies) fails identically |
| this unit's tree changes | an older peer's instrumented gui binary fails identically |
| display power state | `pmset` shows the display off from 10:05 UTC until 19:11:53 UTC — and `Ready` **was** delivered at 18:58:57 UTC inside that window, so the display being off is not the differentiator |

**The correlate that survives:** the GUI session restarted at **19:11:53 UTC**
(`ps -o etime` puts `WindowServer`/`loginwindow` at 1:28:25 when the failing runs were taken,
matching the `pmset` "Display is turned on" event; "Display is turned off" follows at
19:22:04). The pinned S8 artifact was rendered at **18:58:57 UTC**, twelve minutes *before*
that restart; every run after it — four acceptor trials, the 94 s run, the instrumented legs of
this segment, and the TAU-C6-era peer-binary probe — fails to reach `Ready`. The mechanism
*below* tao is not measured (a non-bundled process failing to finish launching in a restarted
session is the shape of it), so this is the best-supported explanation, not a proven one.

**Consequence for the ledger:** the 80 lines in groups S10.1/S10.2/S10.11 are
**`host-state-blocked`** — reachable, and *covered* in the pinned §S8 artifact — not
structural waivers. Anything measured in a session in this state will read them as `DA:0`, and
the driver now says which case it is: its closing line prints `with "embedded Agent ready"` or
`without`.


## S10.4 The residual (217 lines) — groups

| group | lines | what is missing |
|---|---|---|
| S10.1 | 3 | the callback's three window calls (`907`–`909`): the callback runs in this session but `get_webview_window("main")` is `None`, so the arms are skipped. **`needs-window`** — and *not* host-state: the pinned §S8 artifact does not have them either (that driver never notified), so they have been uncovered in every artifact so far |
| S10.2 | 29 | the setup-hook / `Ready`-window family this session cannot reach (`209`–`215`, `474`–`478`, `811`, `955`–`975`, `993`–`1039`, minus the covered ones) — **host-state-blocked**, covered in the pinned §S8 artifact |
| S10.11 | 48 | 46 lines of `menu.rs`'s `build_macos_menu` (called from the same setup hook) — **host-state-blocked**, covered in the pinned §S8 artifact — plus 2 continuation lines (`menu.rs:41`, `:48`, the `)?)` continuations §S8.2 already lists) that are **`attribution-artifact`** and uncovered in every artifact |
| S10.3 | 11 | a real **menu click** (`on_menu_event` body) — AppKit posts it |
| S10.4 | 11 | a real **window close request** (`on_window_event` body) — AppKit posts it |
| S10.5 | 11 | the **failure arms** of real calls that all succeed on this host (menu setup failure, window show, window absent, terminal bind, agent startup) |
| S10.6 | 1 | an **Agent event reaching the real emitter** |
| S10.7 | 11 | `RunEvent::ExitRequested` — `tauri::ExitRequestApi` has private fields and one construction site |
| S10.8 | 3 | the **mobile entry point**, no desktop caller |
| S10.9 | 36 | the **launcher process** and the bin's `argv` arms (`launch.rs`, `main.rs`) — a different entry point |
| S10.10 | 9 | 8 test-internal invariants in closure bodies a test body cannot enter, plus **1 `headless-lib-truncation`** (`src/runtime.rs:46`: covered only by the headless `lib` target, which this leg's SIGTERM killed — §S11.7.4 — so its absence is an artefact of the truncated run, not run-to-run noise). The group shrank from 12 because `runtime.rs:64`–`66`, which Segment 9 listed here, are covered in this artifact |
| S10.12 | 11 | `cfg(all(feature = "gui", not(test)))` blocks — `unreachable-by-construction` |
| S10.13 | 3 | the shutdown deadline arm (its budget knob is in a file outside this scope) |
| S10.15 | 2 | the `freeze_tasks` race pair |
| S10.16 | 1 | a block-end brace — `attribution-artifact` |
| S10.17 | 3 | client-stack arms (`future_login.rs`) |
| S10.18/19 | 1 + 7 | the `APP_HANDLE` arm and the OS thread-refusal arms |
| S10.20 | 10 | real **sleep/wake/power-off notifications** (`macos_power.rs`) — the OS posts them |
| S10.21 | 5 | the `flock` re-acquire window |
| S10.22 | 1 | the Linux-only helper arm — `platform-unmeasured` |
| **total** | **217** | |

**Waived-versus-open arithmetic, summing to the measured residual:**

| class | lines |
|---|---|
| `host-state-blocked` (S10.2 + 46 of S10.11 — **not waived**, covered in the pinned §S8 artifact, expected back in a session that delivers `Ready`) | 75 |
| `needs-window` (S10.1 — the callback runs but `get_webview_window("main")` is `None`; uncovered in every artifact so far) | 3 |
| real-OS-event and alternate-entry-point families (S10.3, S10.4, S10.5, S10.6, S10.7, S10.8, S10.9, S10.13, S10.15, S10.20, S10.21) | 104 |
| `unreachable-by-construction` (8 test-internal invariants of S10.10, S10.12) | 19 |
| `headless-lib-truncation` (`src/runtime.rs:46`, S10.10; §S11.7.4 measured the mechanism: it is covered only by the headless `lib` target, which was SIGTERM-killed in this leg, so its absence here is an artefact of a truncated run) | 1 |
| `attribution-artifact` (S10.16 + `menu.rs:41`/`:48` of S10.11) | 3 |
| client-stack arms and the `APP_HANDLE` / thread-refusal arms (S10.17, S10.18/19) | 11 |
| `platform-unmeasured` (S10.22) | 1 |
| **total** | **217** |

**Correction (Segment 11 verified this against the two artifacts, see §S11.4).** Segment 10 first
recorded all 80 lines of S10.1/S10.2/S10.11 as `host-state-blocked`: **that was wrong for 5 of
them.** The set difference against the pinned artifact shows 3 of S10.1 (`lib.rs:907`–`909`) and
2 of S10.11 (`menu.rs:41`, `:48`) were *already* uncovered in §S8's 150, so they are not this
session's loss — they are `needs-window` and `attribution-artifact`. It also shows the 76-line
gross loss is `46 + 29 + 1`: the odd one out is `src/runtime.rs:46`, which flipped to uncovered
against the pinned artifact with no source change — the known coverage-nondeterminism family,
**not** a `Ready` effect. So the true host-state-blocked count is **75**, and with the 9 gained
callback lines the identity is `150 − 9 + 76 = 217`.

Against the pinned artifact the arithmetic is exact in both directions — **76 lines lost, 9
lines gained**:

* **gained (9):** `src/lib.rs:906`, `910`, `916`, `917`, `919`, `920`, `921`, `923`, `924` —
  the callback's nine non-window lines. This is this unit's own coverage and the pinned §S8
  artifact (taken before the driver could notify) does not have it.
* **lost (76):** `src/menu.rs` 46 + `src/lib.rs` 29 (the two families that are genuinely
  `host-state-blocked`) + `src/runtime.rs:46` (**one line that flipped to uncovered with no
  source change** — the coverage-nondeterminism family, not a `Ready` effect).
  `150 − 9 + 76 = 217`.
* versus Segment 9: `S10 \ S9 = ∅` and `S9 \ S10 = {src/runtime.rs:64,65,66}` — the reworked
  driver's unconditional notification is worth 3 lines net on this host, and no line regressed.

### Per-line index of this segment's residual (217 rows)

| line | group |
|---|---|
| `src/bin/futureos-headless.rs:11` | S10.22 |
| `src/desktop_shutdown.rs:221` | S10.12 |
| `src/desktop_shutdown.rs:222` | S10.12 |
| `src/desktop_shutdown.rs:223` | S10.12 |
| `src/desktop_shutdown.rs:256` | S10.13 |
| `src/desktop_shutdown.rs:257` | S10.13 |
| `src/desktop_shutdown.rs:258` | S10.13 |
| `src/desktop_shutdown.rs:267` | S10.12 |
| `src/desktop_shutdown.rs:268` | S10.12 |
| `src/desktop_shutdown.rs:269` | S10.12 |
| `src/desktop_shutdown.rs:270` | S10.12 |
| `src/desktop_shutdown.rs:271` | S10.12 |
| `src/desktop_shutdown.rs:272` | S10.12 |
| `src/desktop_shutdown.rs:273` | S10.12 |
| `src/desktop_shutdown.rs:274` | S10.12 |
| `src/desktop_shutdown.rs:460` | S10.16 |
| `src/desktop_shutdown.rs:499` | S10.15 |
| `src/desktop_shutdown.rs:500` | S10.15 |
| `src/desktop_shutdown.rs:501` | S10.10 |
| `src/desktop_shutdown/coordinator.rs:50` | S10.18 |
| `src/desktop_shutdown/coordinator.rs:165` | S10.19 |
| `src/desktop_shutdown/coordinator.rs:166` | S10.19 |
| `src/desktop_shutdown/coordinator.rs:183` | S10.19 |
| `src/desktop_shutdown/coordinator.rs:184` | S10.19 |
| `src/desktop_shutdown/coordinator.rs:185` | S10.19 |
| `src/desktop_shutdown/coordinator.rs:186` | S10.19 |
| `src/desktop_shutdown/coordinator.rs:187` | S10.19 |
| `src/future_login.rs:354` | S10.17 |
| `src/future_login.rs:355` | S10.17 |
| `src/future_login.rs:482` | S10.17 |
| `src/instance.rs:164` | S10.21 |
| `src/instance.rs:165` | S10.21 |
| `src/instance.rs:166` | S10.21 |
| `src/instance.rs:167` | S10.21 |
| `src/instance.rs:168` | S10.21 |
| `src/launch.rs:330` | S10.9 |
| `src/launch.rs:331` | S10.9 |
| `src/launch.rs:332` | S10.9 |
| `src/launch.rs:333` | S10.9 |
| `src/launch.rs:334` | S10.9 |
| `src/launch.rs:338` | S10.9 |
| `src/launch.rs:339` | S10.9 |
| `src/launch.rs:340` | S10.9 |
| `src/launch.rs:341` | S10.9 |
| `src/launch.rs:342` | S10.9 |
| `src/launch.rs:343` | S10.9 |
| `src/launch.rs:344` | S10.9 |
| `src/launch.rs:345` | S10.9 |
| `src/launch.rs:346` | S10.9 |
| `src/launch.rs:347` | S10.9 |
| `src/lib.rs:209` | S10.2 |
| `src/lib.rs:210` | S10.2 |
| `src/lib.rs:211` | S10.2 |
| `src/lib.rs:212` | S10.2 |
| `src/lib.rs:213` | S10.2 |
| `src/lib.rs:214` | S10.2 |
| `src/lib.rs:215` | S10.2 |
| `src/lib.rs:474` | S10.2 |
| `src/lib.rs:475` | S10.2 |
| `src/lib.rs:476` | S10.2 |
| `src/lib.rs:477` | S10.2 |
| `src/lib.rs:478` | S10.2 |
| `src/lib.rs:811` | S10.2 |
| `src/lib.rs:842` | S10.6 |
| `src/lib.rs:888` | S10.8 |
| `src/lib.rs:889` | S10.8 |
| `src/lib.rs:890` | S10.8 |
| `src/lib.rs:907` | S10.1 |
| `src/lib.rs:908` | S10.1 |
| `src/lib.rs:909` | S10.1 |
| `src/lib.rs:932` | S10.3 |
| `src/lib.rs:933` | S10.3 |
| `src/lib.rs:934` | S10.3 |
| `src/lib.rs:935` | S10.3 |
| `src/lib.rs:936` | S10.3 |
| `src/lib.rs:937` | S10.3 |
| `src/lib.rs:941` | S10.3 |
| `src/lib.rs:942` | S10.3 |
| `src/lib.rs:943` | S10.3 |
| `src/lib.rs:945` | S10.3 |
| `src/lib.rs:950` | S10.3 |
| `src/lib.rs:955` | S10.2 |
| `src/lib.rs:956` | S10.4 |
| `src/lib.rs:957` | S10.2 |
| `src/lib.rs:958` | S10.2 |
| `src/lib.rs:960` | S10.4 |
| `src/lib.rs:962` | S10.4 |
| `src/lib.rs:963` | S10.4 |
| `src/lib.rs:964` | S10.4 |
| `src/lib.rs:965` | S10.4 |
| `src/lib.rs:967` | S10.4 |
| `src/lib.rs:968` | S10.4 |
| `src/lib.rs:969` | S10.4 |
| `src/lib.rs:970` | S10.4 |
| `src/lib.rs:971` | S10.4 |
| `src/lib.rs:974` | S10.2 |
| `src/lib.rs:975` | S10.2 |
| `src/lib.rs:993` | S10.2 |
| `src/lib.rs:995` | S10.2 |
| `src/lib.rs:1004` | S10.5 |
| `src/lib.rs:1005` | S10.2 |
| `src/lib.rs:1006` | S10.5 |
| `src/lib.rs:1007` | S10.5 |
| `src/lib.rs:1008` | S10.2 |
| `src/lib.rs:1010` | S10.2 |
| `src/lib.rs:1011` | S10.2 |
| `src/lib.rs:1031` | S10.2 |
| `src/lib.rs:1032` | S10.2 |
| `src/lib.rs:1033` | S10.5 |
| `src/lib.rs:1034` | S10.2 |
| `src/lib.rs:1035` | S10.5 |
| `src/lib.rs:1036` | S10.5 |
| `src/lib.rs:1037` | S10.5 |
| `src/lib.rs:1038` | S10.2 |
| `src/lib.rs:1039` | S10.2 |
| `src/lib.rs:1227` | S10.5 |
| `src/lib.rs:1252` | S10.5 |
| `src/lib.rs:1253` | S10.5 |
| `src/lib.rs:1256` | S10.5 |
| `src/lib.rs:1267` | S10.7 |
| `src/lib.rs:1268` | S10.7 |
| `src/lib.rs:1270` | S10.7 |
| `src/lib.rs:1271` | S10.7 |
| `src/lib.rs:1272` | S10.7 |
| `src/lib.rs:1273` | S10.7 |
| `src/lib.rs:1275` | S10.7 |
| `src/lib.rs:1276` | S10.7 |
| `src/lib.rs:1277` | S10.7 |
| `src/lib.rs:1278` | S10.7 |
| `src/lib.rs:1279` | S10.7 |
| `src/lib.rs:1727` | S10.10 |
| `src/lib.rs:1906` | S10.10 |
| `src/lib.rs:1907` | S10.10 |
| `src/macos_power.rs:21` | S10.20 |
| `src/macos_power.rs:22` | S10.20 |
| `src/macos_power.rs:23` | S10.20 |
| `src/macos_power.rs:24` | S10.20 |
| `src/macos_power.rs:26` | S10.20 |
| `src/macos_power.rs:27` | S10.20 |
| `src/macos_power.rs:29` | S10.20 |
| `src/macos_power.rs:30` | S10.20 |
| `src/macos_power.rs:31` | S10.20 |
| `src/macos_power.rs:32` | S10.20 |
| `src/main.rs:9` | S10.9 |
| `src/main.rs:29` | S10.9 |
| `src/main.rs:33` | S10.9 |
| `src/main.rs:34` | S10.9 |
| `src/main.rs:38` | S10.9 |
| `src/main.rs:39` | S10.9 |
| `src/main.rs:41` | S10.9 |
| `src/main.rs:42` | S10.9 |
| `src/main.rs:43` | S10.9 |
| `src/main.rs:44` | S10.9 |
| `src/main.rs:46` | S10.9 |
| `src/main.rs:47` | S10.9 |
| `src/main.rs:48` | S10.9 |
| `src/main.rs:51` | S10.9 |
| `src/main.rs:58` | S10.9 |
| `src/main.rs:70` | S10.9 |
| `src/main.rs:71` | S10.9 |
| `src/main.rs:72` | S10.9 |
| `src/main.rs:73` | S10.9 |
| `src/main.rs:74` | S10.9 |
| `src/main.rs:75` | S10.9 |
| `src/menu.rs:26` | S10.11 |
| `src/menu.rs:27` | S10.11 |
| `src/menu.rs:28` | S10.11 |
| `src/menu.rs:29` | S10.11 |
| `src/menu.rs:31` | S10.11 |
| `src/menu.rs:32` | S10.11 |
| `src/menu.rs:33` | S10.11 |
| `src/menu.rs:34` | S10.11 |
| `src/menu.rs:35` | S10.11 |
| `src/menu.rs:36` | S10.11 |
| `src/menu.rs:37` | S10.11 |
| `src/menu.rs:38` | S10.11 |
| `src/menu.rs:39` | S10.11 |
| `src/menu.rs:40` | S10.11 |
| `src/menu.rs:41` | S10.11 |
| `src/menu.rs:42` | S10.11 |
| `src/menu.rs:43` | S10.11 |
| `src/menu.rs:44` | S10.11 |
| `src/menu.rs:45` | S10.11 |
| `src/menu.rs:46` | S10.11 |
| `src/menu.rs:47` | S10.11 |
| `src/menu.rs:48` | S10.11 |
| `src/menu.rs:49` | S10.11 |
| `src/menu.rs:51` | S10.11 |
| `src/menu.rs:52` | S10.11 |
| `src/menu.rs:53` | S10.11 |
| `src/menu.rs:54` | S10.11 |
| `src/menu.rs:55` | S10.11 |
| `src/menu.rs:56` | S10.11 |
| `src/menu.rs:57` | S10.11 |
| `src/menu.rs:58` | S10.11 |
| `src/menu.rs:59` | S10.11 |
| `src/menu.rs:61` | S10.11 |
| `src/menu.rs:62` | S10.11 |
| `src/menu.rs:63` | S10.11 |
| `src/menu.rs:65` | S10.11 |
| `src/menu.rs:66` | S10.11 |
| `src/menu.rs:67` | S10.11 |
| `src/menu.rs:68` | S10.11 |
| `src/menu.rs:69` | S10.11 |
| `src/menu.rs:70` | S10.11 |
| `src/menu.rs:72` | S10.11 |
| `src/menu.rs:73` | S10.11 |
| `src/menu.rs:74` | S10.11 |
| `src/menu.rs:75` | S10.11 |
| `src/menu.rs:76` | S10.11 |
| `src/menu.rs:77` | S10.11 |
| `src/menu.rs:78` | S10.11 |
| `src/runtime.rs:46` | S10.10 |
| `src/runtime.rs:248` | S10.10 |
| `src/runtime.rs:249` | S10.10 |
| `src/runtime.rs:251` | S10.10 |
| `src/runtime.rs:252` | S10.10 |

## S10.5 The callback's twelve lines, per line

Both notification shapes are sent, and both children exit inside the plugin's `setup`
(`std::process::exit(0)`, §S6.7.1) — measured at **0.16 s** each in this run — and the
forwarded re-pair is acknowledged on the reply file with
`{"error":"the remote start ended in Failed (AccountAuthorization)"}`, which is the
attribution that the callback ran rather than that a child exited.

| line | count | what it is |
|---|---|---|
| `906` | 8 | the plugin's `init` closure is entered |
| `910` | 8 | the `forwarded_request` match |
| `916`, `917` | 2, 8 | the `spawn_remote_re_pair` arm and its `RePair` delivery choice |
| `919`, `920`, `921` | 2, 1, 1 | both `argv` shapes distinguished (`reply: None` / `Some(path)`) |
| `923`, `924` | 6, 8 | the terminal and file delivery paths |
| `907`–`909` | 0 | the three **window** calls (`unminimize`, `show`, `set_focus`) — they need `get_webview_window("main")` to be `Some`, which is the window `Ready` creates; **host-state-blocked**, not unreachable |

So **9 of the 12 lines** this target exists to cover are covered in this session's artifact,
and the remaining 3 are the same host-state dependency as §S10.3.

## S10.6 Gates, tripwire, and what this segment claims

Run on the frozen tree (the source edits preceded the measurement, as PLAN 5.1 requires):

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **0** | clean crate-wide (0 diffs); the driver file is clean on its own too |
| clippy, this target | `cargo clippy --test gui_main_thread_drive -- -D warnings` | **0** | 0 errors, 0 warnings — the reworked driver lints clean |
| clippy, all targets (gui) | `cargo clippy --all-targets -- -D warnings` | **101** | **one** error, `src/store/threads.rs:1717` (`cloned_ref_to_slice_refs`) — a **peer's** test file, outside this unit's scope; reported, not touched |
| clippy, all targets (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **101** | the same single peer error |
| the driver, uninstrumented | `cargo test --test gui_main_thread_drive` | **0** | runs end to end: both children exit 0, the reply file is answered, the marker is the last line, 0 leaked roots, 19.5 s in this host state |
| gui leg (the measured run) | §S10.2 | **0** | `2076 passed / 0 failed / 12 ignored` |
| headless leg (the measured run) | §S10.2 | **101** | five `remote_peer` failures, named in §S10.2 — outside this unit |

**PLAN §5.2 tripwire.**

```bash
python3 .future/cov100/reconcile.py --registry … \
    --scope desktop/src-tauri/src/lib.rs --scope desktop/src-tauri/src/main.rs \
    --scope desktop/src-tauri/src/menu.rs --scope desktop/src-tauri/src/launch.rs \
    --scope desktop/src-tauri/src/instance.rs --scope desktop/src-tauri/src/runtime.rs \
    --scope desktop/src-tauri/src/host_environment.rs \
    --scope desktop/src-tauri/src/macos_power.rs \
    --scope desktop/src-tauri/src/desktop_shutdown.rs \
    --scope desktop/src-tauri/src/desktop_shutdown/ \
    --scope desktop/src-tauri/src/future_login.rs --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 10" --label tauri-shell-seg10
    → uncovered lines=217  mentioned=217  MISSING=0  (exit 0)
```

**What this segment claims, exactly.** The driver no longer depends on the host delivering
`Ready` for the coverage it exists to produce — measured: the nine callback lines are covered
in the same artifact whose total is **217**. The 67-line net regression against the pinned
artifact is **not** this segment's structural residual: 76 lines are
`host-state-blocked` (covered in the pinned §S8 artifact, which was taken 12 minutes before
this session's `WindowServer` restarted) and 9 are this unit's own new coverage. The unit is
therefore at **9 of 12** callback lines with **0 open** rows of its own, and its pinned
residual for a healthy session remains **150**.

## S10.7 Reproduction

```bash
 # 1. the driver alone (idle host that delivers Ready: ~5-8 s; this session: ~19.5 s)
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --test gui_main_thread_drive
 # last line must be:  gui_main_thread_drive: driver completed
 # the line before it says "with" or "without" the readiness evidence — record which

 # 2. the measurement (§S10.2) with its completeness checks
bash /tmp/tau-c10-measure.sh
grep -c "test store::threads" /tmp/tau-c10-measure.log
grep -E "^gui_main_thread_drive:" /tmp/tau-c10-measure.log

 # 3. the ready-state probe (§S10.3): a listener that never accepts
 #    lsof -p <pid> -a -iTCP -sTCP:LISTEN   then   curl -m 4 http://127.0.0.1:<port>/
 #    http_code=000 means the Ready arm has not run

 # 4. the ledger tools
python3 /tmp/c10-index.py /tmp/tauri-shell-lcov-c10union.info
python3 /tmp/c10-arith.py
```

# Segment 11 — the re-measurement in a `Ready`-delivering session is BLOCKED by the host, and the cause is named at the launchd-domain level

Segment 10 asked for one thing that matters: re-measure in a session that delivers
`RunEvent::Ready` and watch the 75 `host-state-blocked` lines come back. This segment shows
that **no run in this environment can do that**, names the cause one level below §S10.3's
correlate, and corrects five lines of Segment 10's own classification. No source file changed.

## S11.1 The driver still reports `without`, and now the level below it is measured

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --test gui_main_thread_drive     # exit 0
 #   → gui_main_thread_drive: driver completed without "embedded Agent ready"; listener_up=true elapsed=19.9s
 #   → gui_main_thread_drive: driver completed
```

`without` again, 19.9 s. The chain §S10.3 pinned (`Ready` ← tao `StartCause::Init` ←
`AppState::launched` ← `applicationDidFinishLaunching:`) stops one level earlier than the
WindowServer-restart correlate suggested, and this segment measures where:

| probe | command | result |
|---|---|---|
| can a process spawned here reach the live WindowServer? | `/usr/sbin/screencapture -x /tmp/c10-probe.png` | **exit 1**, `could not create image from display` |
| is the user's **GUI** launchd domain reachable from here? | `launchctl print gui/$(id -u)` | **`Could not print domain: 125: Domain does not support specified action`** |
| is the **user** domain reachable (control)? | `launchctl print user/$(id -u)` | prints `user/501 = { type = user … }` |
| can LaunchServices start an app in the current session? | `open -a Terminal /tmp/c10-probe.sh` | **exit 1**, `RBSRequestErrorDomain Code=5 "Launch failed."` caused by `OSLaunchdErrorDomain Code=125 "Domain does not support specified action"` — no probe file was produced |
| can launchd be asked to spawn one (the "no new process" route)? | `launchctl submit -l com.futureos.covprobe -- …` | **exit 134** (SIGABRT — the subcommand is gone) |
| the privileged route (`launchctl asuser`) | `sudo -n true` | `sudo: a password is required` |
| when did this agent start, relative to the session? | `ps -o lstart -p 16714` | `/opt/homebrew/bin/future` started **2026-10-16 18:18:29 local = 10:18:29 UTC**, i.e. **6 h 53 min before** `WindowServer`/`loginwindow` restarted at **19:11:51 UTC** |

**CORRECTION (supervisor, 2026-10-17, measured live — see `docs/testing/waiver-ledger.md`
*Environment-attributed numbers*).** The two timestamps in the row above do not survive a `ps` check,
and the interval is wrong by three hours:

```
$ ps -eo pid,lstart,comm | grep 16714
16714 五 10月/16 17:18:29 2026   /opt/homebrew/bin/future
$ ps -eo pid,lstart,comm | grep -E 'WindowServer|loginwindow'
58900 六 10月/17 03:11:51 2026   …/WindowServer
58905 六 10月/17 03:11:51 2026   …/loginwindow.app/Contents/MacOS/loginwindow
$ date; date -u        # 2026年10月17日 09:18:28 CST / 01:18:28 UTC  (so local = UTC+8)
```

So the agent started **17:18:29 local = 09:18:29 UTC** (not 18:18:29 / 10:18:29), the GUI session
restarted **03:11:51 local = 19:11:51 UTC**, and the agent **precedes** it by **9 h 53 min**, not
6 h 53 min. The record's own `19:11:51 UTC` is right; its agent line is shifted one hour, which is why
its interval disagreed with the other reading of the same facts (§S10.3 and one segment-14 delivery
each quoted a different figure for the same comparison).

**Nothing the row rests on changes**: the mechanism is the **ordering** — the agent predates the
restart, so this process and everything it spawns belong to the pre-restart GUI-session context, and
`gui/501` is unreachable (`error 125`). That ordering was re-confirmed live today, with the same agent
pid 16714 still serving. The correction matters only so a reader comparing timestamps uses numbers that
match `ps`.


So: this agent and every process it spawns belong to the **pre-restart** GUI session context.
The current session's `gui/501` domain is not reachable from here (`error 125` on both read
and launch), which is exactly the handshake AppKit needs to finish launching an application —
hence `Ready` never fires, `screencapture` has no display, and `open` cannot launch anything.
Roles of the evidence are distinct: the domain error **125** is the mechanism; the agent's
start time **preceding** the restart is what explains why this process is in the old context;
the restart itself (§S10.3) is when the split appeared.

## S11.2 Why the earlier falsifications still stand, with the labels verified

Segment 10 falsified "display power state" from an unlabeled read of `pmset` (my `sed` had
stripped the `Display is turned on/off` label). Re-read **with labels**, the conclusion is the
same and the read is now sound:

| event (local, UTC+8) | UTC | label |
|---|---|---|
| 2026-10-16 17:22:36 | 09:22:36 | Display is turned **on** |
| 2026-10-16 18:05:34 | 10:05:34 | Display is turned **off** |
| 2026-10-17 03:11:53 | 19:11:53 | Display is turned **on** |
| 2026-10-17 03:22:04 | 19:22:04 | Display is turned **off** |

The pinned Segment-8 artifact was rendered at **18:58:57 UTC** — the display was **off** then
(off since 10:05:34) and `Ready` **was** delivered. The display is off again now, as it was
then. So display state is not the differentiator; the `19:11:51` session restart is. The load
and fd-limit falsifications are unaffected.

## S11.3 What this means for the campaign (the actionable part)

* **The pinned 150 for this scope is currently unreproducible by any run in this
  environment** — including the frozen pass, which runs from the same agent context. A
  measurement of this track taken now will read **217** (216–218 with the
  `coverage-nondeterministic` line), and that is not a regression: 75 of those lines are
  covered in §S8.3's artifact.
* **The unblock is a host action, not a code change:** restart the `future` agent (pid 16714)
  — or start one — from a terminal that is already inside the current GUI session, so the new
  agent inherits the live `gui/501` domain. Nothing in this unit's write set can do it: the
  domain is unreachable (125), `launchctl submit` is gone, and `launchctl asuser` needs root.
* **A two-line precondition check for whoever runs next:** first, if
  `cargo test --test gui_main_thread_drive` ends `with "embedded Agent ready"`, the session is
  healthy and the measurement is worth taking; if it ends `without`, the measurement will
  read ~217 and should not be taken as this unit's residual. Second, even a `without` run must
  show the notification hand-off working — `… child exited … after 0.xxxs` and
  `the forwarded re-pair was answered` — or the nine callback lines are missing too (§S11.8).
  The driver prints all three facts.

## S11.4 Correction to Segment 10's classification (5 lines, verified by set difference)

Re-checking Segment 10's group table against the two artifacts shows three things I got wrong
when I wrote it, all now corrected in §S10.4 and in the header:

1. **3 lines of S10.1 (`lib.rs:907`–`909`) are not host-state-blocked.** They are uncovered in
   the pinned §S8 artifact too — that driver never sent a notification at all (the notify step
   arrived in Segment 9), so these window arms have been uncovered in every artifact so far.
   They are `needs-window`: the callback runs here, but `get_webview_window("main")` is `None`.
2. **2 lines of S10.11 (`menu.rs:41`, `:48`) are not host-state-blocked either** — they are the
   `)?)` continuation lines §S8.2 already lists as `attribution-artifact`.
3. **1 of the 76 gross-lost lines is not a `Ready` effect at all:** `src/runtime.rs:46` flipped
   to uncovered against the pinned artifact with no source change. §S11.4 first called this
   `coverage-nondeterministic`; **§S11.7.4 replaced that with a measured mechanism** — the line
   is covered only by the headless `lib` target, which the Segment-9 and Segment-10 legs were
   killed before finishing (SIGTERM), so the pinned artifact has it and those two do not. It is
   a deterministic truncation artefact, not run-to-run noise. The gross loss is `46 + 29 + 1`.

The corrected split of the 217 — **75** `host-state-blocked`, **3** `needs-window`, **104**
real-OS-event/alternate-entry, **19** `unreachable-by-construction`, **1**
`headless-lib-truncation` (`runtime.rs:46`, §S11.7.4), **3** `attribution-artifact`, **11**
client-stack/`APP_HANDLE`, **1** `platform-unmeasured` — sums to 217 with **0 open**. The
per-line index in §S10.4 was already derived from the S9 ledger and needs no change: only the
class labels moved.

## S11.5 Gates and tripwire on the unchanged tree

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check` | **1** | **one** diff, `src/remote/test_support.rs:1792` — a **peer-owned** file (`src/remote/**` belongs to the remote unit and is outside this record's scope). **Reported, not touched.** Neither the driver target nor any file in this unit's write set has a diff |
| clippy, this target | `cargo clippy --test gui_main_thread_drive -- -D warnings` | **0** | 0 errors, 0 warnings |
| clippy, all targets (gui) | `cargo clippy --all-targets -- -D warnings` | **0** | clean — the `src/store/threads.rs:1717` peer lint reported in §S10.6 is **fixed upstream**, so this gate is green now |
| clippy, all targets (headless) | `cargo clippy --no-default-features --features headless --all-targets -- -D warnings` | **0** | clean |
| the driver | `cargo test --test gui_main_thread_drive` | **0** | `without` readiness, 19.9 s, marker last, 0 leaked roots |

Tripwire unchanged and re-run against the same artifact:

```bash
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-c10union.info \
    --scope desktop/src-tauri/src/lib.rs … --scope desktop/src-tauri/src/bin/ \
    --doc docs/testing/module-tauri-shell.md --section "Segment 10" --label tauri-shell-seg10
 #   → uncovered lines=217  mentioned=217  MISSING=0  (exit 0)
```

No source file changed in this segment, so the measured artifact and the ledger are the same
revision as Segment 10's, and no new measurement was needed or taken.

**Caveat on these gate runs:** under this host's load (13 with eight peer `cargo` processes
alive) a full `cargo clippy --all-targets` took ~10 minutes of wall clock and my first two
attempts were killed by the tool's own timeouts before finishing. The numbers above are from
the run that completed, reported as-is.

## S11.6 The two decisions this segment carried, and where they stand

1. **`desktop_shutdown.rs:256`–`258` (3 lines, group S10.13) — RESOLVED, see §S12.1.**
   *As this segment recorded it:* the todo asked for
   `desktop/src-tauri/src/desktop_shutdown/coordinator.rs` in this unit's write set (or its
   owner exposing `begin_with_budget` as `pub(super)`), and no ruling existed yet. With the
   production `TOTAL_BUDGET = 20 s` the arm cannot fire: the awaited publish is capped at
   `DISCONNECT_NOTICE_TIMEOUT = 750 ms` (§S6.7.3). The supervisor's answer (grant of
   2026-10-17) was **test-only write-set extension to `coordinator.rs`**, not the `pub(super)`
   change, and §S12.1 is the resulting test.
2. **The menu-click / window-close driver (22 lines, groups S10.3/S10.4) — NOT landed, and it
   buys nothing today.** `dispatch2` + `objc2-app-kit` on the main queue could drive
   `NSApp.mainMenu performActionForItemAtIndex` and `NSWindow performClose`, but both need the
   menu and window to exist, i.e. `Ready`, which this session does not deliver. Landing it
   would add a dev-dependency and a second handler for zero coverage in the current
   environment, so it stays a named, unexecuted proposal.

## S11.7 Forensics: the headless leg's `--lib` target is killed, and what that costs the numbers

The supervisor read `/tmp/tau-c10-measure.log` and found two things that do not match §S10.2's
report. Both are real, and checking them turned up a third: **the kill is chronic across
measurements, and its cost inside this unit's twelve scopes is exactly one line.**

### S11.7.1 The `--lib` run did not fail — it was terminated by SIGTERM

```
line 6044: test remote_peer::tests::only_reads_and_the_two_conversation_commands_are_retryable ... ok
line 6057: test remote_peer::tests::the_host_learns_what_connected_to_it ... error: test failed, to rerun pass `--lib`
           Caused by:
             process didn't exit successfully: `…/deps/futureos_lib-aa6cb33e5ce4815c --test-threads=1`
             (signal: 15, SIGTERM: termination signal)
line 6094: error: 1 target failed:
line 6096: error: process didn't exit successfully: `… cargo test … --no-fail-fast -j 4 --no-default-features --features headless -- --test-threads=1` (exit status: 101)
```

**Aborted, not failed**, and the distinction is diagnostic: libtest prints `test result:` only
when a run *completes*, so a killed target prints none, whereas a plain test failure prints
`test result: FAILED. …` and still carries the "how many ran" counts. `signal: 15` (SIGTERM)
also rules out the other two shapes: a panic-abort would be `6 (SIGABRT)` and an OOM kill
`9 (SIGKILL)`. `--no-fail-fast` is what let the leg's remaining three targets run, which is why
the rest of the log looks normal.

**It is not one test doing it.** The same binary died in the Segment-9 measurement
(`/tmp/tau-c9-measure.log:4111`, same `futureos_lib-aa6cb33e5ce4815c`) — but *at a different
place in the suite*: c9 died while running `agent_bridge::persist::tests::…` (early), c10 died
while running `remote_peer::tests::…` (late). No test in the repository kills by name (the
only `pkill`/`killall` commentary and `libc::raise(SIGTERM)` calls in the tree are in `tui/`,
a different crate and a different test binary), and the campaign's own scripts
(`.future/cov100/*.sh|*.py`) contain no kill. So the signal comes from outside the target, and
neither "a specific test" nor "this unit's fixtures" explains it.

**Standalone, it does not reproduce.** Running the same target alone — `cargo test
--no-default-features --features headless --lib -- --test-threads=1`, launched 21:14:04 UTC
with no instrumented run active — **completed and printed its summary**:

```
test result: FAILED. 1659 passed; 3 failed; 12 ignored; 0 measured; 0 filtered out; finished in 442.97s
exit 101 (the ordinary "3 tests failed" exit — no `signal:` line anywhere in the log)
```

Those three failures are a *different* set from the instrumented leg's four, and none of them
is `remote_peer`: `headless::tests::a_failing_builtin_skill_bootstrap_is_ignored`,
`headless::tests::the_builtin_skill_bootstrap_runs_the_cli`,
`tasks::tests::the_headless_loop_runs_a_queued_request_it_is_woken_for` — the environment-
sensitive families, not the port-8022 one. So the kill belongs to the *two-leg instrumented
run in a loaded environment* (both measurements, at different points, with eight to fifteen
concurrent `cargo` processes on the host), not to the suite, the target, or this unit's code.
**The killer itself is not identified** — nothing in the log or in this repository's tooling
names it, and I am not going to invent a mechanism; what is established is that it is an
external SIGTERM that does not occur in an unloaded standalone run.
The decisive follow-up for whoever owns the harness: **run each leg in the foreground and
block on it**, and treat a leg whose `Running` count exceeds its summary count as incomplete
(the guard already does) — rather than trusting a leg's exit code.

### S11.7.2 Completeness accounting, so the next reader can check it

| leg | `Running` lines | `test result:` summaries | bare `ok` | bare `FAILED` | verdict |
|---|---|---|---|---|---|
| gui | 4 | 3 **+ the driver's marker** (a `harness = false` target prints none by design) | 246 | **0** | **complete**; the lib summary is present (`2076 passed; 0 failed; 12 ignored`, 524.61 s) |
| headless | 4 | **3** — the `--lib` target has none | 183 | **4** | **truncated at the target level** (`--lib` killed; main 2/0, bin 1/0, headless_cli 12/0 complete) |

Read that against the guard the campaign already has: `measure-final.sh` counts `Running`
lines against `test result:` lines and flags a track as truncated — for the headless leg it
would be right to, and this section is that flag's explanation rather than a hole in it.

### S11.7.3 Four failures, not five — my §S10.2 list was wrong in two entries

The per-test verdicts interleave with tracing output, so a verdict lands on a line *after* its
test's name line. The four `FAILED` lines (5824, 5833, 5848, 5859) therefore belong to the
four tests whose name lines precede them:

| # | test (verdict `FAILED`) |
|---|---|
| 1 | `remote_peer::runtime_tests::connect_all_resumes_a_host_the_user_had_stopped` |
| 2 | `remote_peer::runtime_tests::connecting_an_unknown_host_is_an_error_not_an_invention` |
| 3 | `remote_peer::runtime_tests::connecting_replaces_an_existing_retry_loop` |
| 4 | `remote_peer::runtime_tests::disconnect_drops_the_connection_but_keeps_the_pairing` |

This is exactly the supervisor's list. My §S10.2 report had **five** entries, wrong twice over:
it named `remote_peer::runtime_tests::disconnecting_stops_the_event_stream`, which **passed**
(its `ok` is the line after its tracing block — I read the verdict of the *previous* test as
this test's), omitted `connect_all_resumes_a_host_the_user_had_stopped`, and listed
`remote_peer::tests::the_host_learns_what_connected_to_it`, which is not a test failure at all
but the test that was **running when SIGTERM arrived** — cargo names it in the target-level
`Caused by:` block. So: **4 test-level failures + 1 target-level abort**, and §S10.2's list is
corrected accordingly. All four are `remote_peer`, outside the twelve scopes, and this unit's
driver test ran and passed in the green gui leg.

### S11.7.4 The kill is a *cause* I previously mis-attributed — and its cost is one line

The pinned Segment-8 artifact came from a run whose headless lib target **completed**
(`/tmp/tau-c8-measure.log:` `test result: ok. 1651 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out; finished in 407.23s`), so the pinned union contains headless-`lib` coverage that
the c9 and c10 unions lack. To size that hole I compared the c8 per-flavour reports: the set of
in-scope lines the headless flavour covers and the gui flavour does not is **70 lines**
(`bin/futureos-headless.rs` 50, `runtime.rs` 12, `main.rs` 7, `desktop_shutdown.rs` 1) — and
**69 of those 70 are covered in the c10 union anyway**, because they come from the headless
*main*, *bin* and *headless_cli* targets, which completed in that leg. Exactly **one** is
uncovered in the c10 union:

| line | c8 gui | c8 headless | c10 gui | c10 headless | c10 union |
|---|---|---|---|---|---|
| `src/runtime.rs:46` | – | **7** | – | **0** | **0** |
| `src/runtime.rs:67` | – | 255 | – | 27 | 27 |

So the 217's composition is affected in **one line**, and that line is the one §S11.4 labelled
`coverage-nondeterministic` ("flipped with no source change"). The better-supported explanation
is now this one: `runtime.rs:46` is covered **only** by the headless `lib` target, which was
killed in the c9 and c10 legs, so its absence is a **deterministic artefact of the truncation**,
not run-to-run noise — and so is the reduced-but-nonzero count of its neighbours
(`runtime.rs:64`–`69`: 255 → 27, 5 → 4). §S11.4's class for it was right that the line is not a
`Ready` effect and wrong about the mechanism; the record now says truncation.

**What this does not change:** the gui leg — which carries this unit's scopes — printed its
summary and was green in both legs' sense (0 test failures), the 217 arithmetic
(`150 − 9 + 76`) stands, and §S10.2's failure list is corrected without touching any number.

### S11.7.5 Standalone check

```bash
cargo test --no-default-features --features headless --lib -- --test-threads=1
```

launched at 21:14:04 UTC with no instrumented run active — `/tmp/c11-standalone.log`,
`/tmp/c11-standalone.out`. Result: **the target completed with its summary**
(`1659 passed; 3 failed; 12 ignored` in 442.97 s, exit 101 from the three failures, no signal),
so the truncation is an artefact of the two-leg instrumented run and not of the suite. Its
three failures are named in §S11.7.1.

## S11.8 The Segment-12 precondition check: still `without`, and the notification hand-off is now failing too

The successor's first instruction is a ~30 s precondition check before any measurement. It was
run twice and answers **stop**. No measurement was taken and **no ledger number changed** in
this segment; the artifact stays `/tmp/tauri-shell-lcov-c10union.info` (217) and §S8.3 stays the
pinned healthy-session number (150).

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
cargo test --test gui_main_thread_drive        # exit 0
```

Verbatim, the closing lines of the first check (`/tmp/c12-pre.log`, 21:36:08 → 21:40:01 UTC,
load 15.04):

```
gui_main_thread_drive: "embedded Agent ready" did not appear within 15s; notifying anyway
gui_main_thread_drive: driver completed without "embedded Agent ready"; listener_up=true elapsed=17.0s
gui_main_thread_drive: driver completed
```

So `Ready` is still absent (§S11.1's blocker), and the driver's own early-exit rule fired
exactly as designed.

**A second observation, new in this segment: the notification hand-off did not complete.** The
same run shows the two children running as applications instead of exiting at the plugin's
`setup`:

| observation | this run (21:36) | the Segment-10 measurement (20:31) |
|---|---|---|
| `bare --re-pair` child | exited 0 after **17.10 s** | exited 0 after **0.16 s** |
| `--re-pair --re-pair-reply` child | exited 0 after **17.16 s** | exited 0 after **0.16 s** |
| reply file | **`no reply file … within 6s (the callback's Reply arm did not finish)`** | `the forwarded re-pair was answered on …` with the AccountAuthorization JSON |

17.1 s is not a hand-off latency: it is this driver's own primary path — its 15 s `READY_BUDGET`
plus the 2 s `SETTLE` — which is what a **companion** process runs when it skips the
notification step. In other words each child became an app of its own (its own marker line is in
the log) instead of exiting inside the plugin, and the parent's listener received nothing, so
its callback never ran. **The mechanism is not established** and the record does not guess one:
it is *not* this unit's code (nothing in the driver changed between the two columns), and the
one thing that differs is the host state (load 15–28 with several peer `cargo`/`clippy`
builds contending — the repeat precondition run was itself blocked for minutes on the target
directory lock held by a peer process).

**Consequence, and why it reinforces the stop rule.** The nine callback lines this unit earned in
Segment 10 come from *this* notification path. If the hand-off is failing, a measurement taken
now would read those nine as `DA:0` **as well** — i.e. ~226 rather than 217 — and that would be
a second environment artefact, not this unit's residual. Anyone who does measure must therefore
check two things, not one: the readiness line (`with`/`without`) **and** the hand-off
(`… child exited … after 0.xxxs` plus `the forwarded re-pair was answered`). Both are printed by
the driver, so both are checkable from the leg's log.

**Scope of this note:** it is evidence about the environment, added to the same record because
the precondition is this unit's own gate. It does not revise the 217, its class split
(§S10.4), the pinned 150, or any `Segment N` ledger.

## S11.9 Segment 13: both checks re-run (both still fail), and the hand-off failure is localized — plus the mitigation options for the supervisor

Segment 13's instruction is a two-check precondition. Both checks were re-run and **both still
fail**; the hand-off failure is now reproducible and localized, and the code changes that could
mitigate it are named here for a decision rather than picked. **No measurement was taken and no
ledger number changed** — the artifact stays `/tmp/tauri-shell-lcov-c10union.info` (217) and the
pinned healthy-session figure stays §S8.3's **150**.

### S11.9.1 Method note: the `cargo` precondition could not run, the binary could

`cargo test --test gui_main_thread_drive` was launched at 22:02:12 UTC and **never started**: for
30 minutes its log held one line — `Blocking waiting for file lock on build directory` — held by
a peer's `cargo`/`clippy` (one pid at 17:30 elapsed, several shorter ones). The checks below were
therefore taken by running the **already-built driver binary directly**
(`target/debug/deps/gui_main_thread_drive-0a9f798f04f91dc3`), which is the same code path minus
`cargo`; the harness adds nothing to these two signals. The blocked run was killed rather than
left queueing. (This is the same contention §S11.7.5 saw from the other side.)

### S11.9.2 CHECK A (readiness): `without` — now the third observation

```
gui_main_thread_drive: "embedded Agent ready" did not appear within 15s; notifying anyway
gui_main_thread_drive: driver completed without "embedded Agent ready"; listener_up=true elapsed=17.0s
gui_main_thread_drive: driver completed
```

Identical in shape to §S11.8. Host state is unchanged: `WindowServer`/`loginwindow` up 2:50:17
(same 19:11:51 UTC restart) and the agent still the pre-restart pid 16714 from 10:18:29 UTC, so
§S11.1's launchd-domain blocker stands and the host action `todo_3917fe72dae3` is still the fix.

### S11.9.3 CHECK B (hand-off): `FAILED`, reproducibly, and the numbers are byte-similar

| run | bare child | forwarded child | reply file |
|---|---|---|---|
| Segment 10 measurement (20:31) | 0.16 s | 0.16 s | **answered** (`{"error":"…Failed (AccountAuthorization)"}`) |
| Segment 12 precondition (21:36) | 17.10 s | 17.16 s | **absent** |
| **Segment 13, direct binary (22:42)** | **17.11 s** | **17.11 s** | **absent** — `no reply file at …/futureos-gui-drive-reply-15784.json within 6s (the callback's Reply arm did not finish)` |

Two independent runs, 66 minutes apart, agree to the hundredth of a second, and each child's own
marker is in the log (`driver completed without "embedded Agent ready"; listener_up=true
elapsed=17.1s`). 17.1 s is this driver's **companion** path (15 s readiness budget + 2 s settle)
and a companion still prints that marker; an early exit inside the plugin prints nothing and
would have been logged by the parent as `child exited … after 0.1xs`. So in these runs the
children **became applications instead of notifying**, their plugin-level `notify_singleton`
took an error path, the parent's callback never received a `RePair`, and no reply file appeared.

**Where it is localized, and what is not established.** The plugin's macOS `setup` exits the
child immediately when `UnixStream::connect(socket)` succeeds
(`tauri-plugin-single-instance-2.4.3/src/platform_impl/macos.rs`, `notify_singleton` → `Ok(_) =>
std::process::exit(0)`), and the parent's own probe *did* connect at the start of the same run
(its closing line says `listener_up=true`). So the two sides disagree about the same path — the
child's notify, which fires ~15 s later (inside `Builder::build`, before my watchdog), fails
while the parent's earlier probe succeeded. A related observation from the isolation test
(§S11.9.4) shows the listener binding itself is unreliable here. **I did not establish the exact
error the child's `connect`/write hit**, and this record does not guess: distinguishing
`ConnectionRefused` from a write error would need the plugin's own `tracing::debug!` output,
and instrumenting it is outside this unit's scope.

**Why it matters, in one number:** the nine callback lines this unit earned in Segment 10 come
from exactly this path. With the hand-off failing, a measurement taken now would read those nine
as `DA:0` as well — ~226 rather than 217 — which is a second environment artefact, not this
unit's residual.

### S11.9.4 Isolation test: the listener bind itself is unreliable in this session

A separate probe (not a coverage measurement, `/tmp/c13b-isolate.sh`) started a primary with
`FUTUREOS_DRIVE_READY_BUDGET_SECS=1` and then ran a second instance of the same binary against
the same root:

* the primary's **own socket never appeared within 30 s** — `lsof -p <pid> -a -U` printed nothing
  and its closing line shows `elapsed=31.7s`, i.e. the full `LISTENER_BUDGET` expired with
  `listener_up=false`;
* the second instance then ran **42.68 s** and printed its own marker, i.e. it claimed
  singletonship (a plugin `connect` failure followed by a bind that could not succeed either).

This is the third symptom of one root cause and it is the same one behind §S11.9.3: in this
session the application's runtime-level work does not progress — `RunEvent::Ready` never fires
(§S11.1) and the plugin's async listener frequently does not bind within its budget. When the
listener is not up, a second instance *cannot* hand off, so children become apps and the callback
never runs. (That run's primary was also visibly slower than earlier ones: even with a 1 s
readiness budget it needed the whole 30 s listener budget.)

### S11.9.5 The mitigation options, with their evidence — for the supervisor, not picked here

Each is a change to `tests/gui_main_thread_drive.rs` (this unit's file), so each needs a decision:

| option | what it would do | evidence for / against |
|---|---|---|
| **(a) retry the notification once** | if a child exits after more than ~5 s (the companion path, not a hand-off) or no reply file appears, spawn the notification again after re-checking the listener | the two outcomes are trivially distinguishable (0.1x s vs 17 s, and marker present vs absent), so a retry is self-checking; costs a second spawn only in the failing case. Does not help if the listener stays down for the whole run |
| **(b) wait for the listener to have *accepted* before spawning** | requires an observable accept signal, which the plugin does not expose; a proxy would be my own connect-and-close, which the plugin's accept loop already consumes harmlessly | the parent's `listener_up` is exactly this probe and it was already `true`, yet the children still failed — so this option's value is unproven, and (a) subsumes it |
| **(c) fail loudly instead of silently earning nothing** | print a distinct line (e.g. `gui_main_thread_drive: hand-off FAILED`) when the children ran the companion path or the reply never came, so a measurement can be discarded automatically instead of reading ~226 as if it were 217 | cheap and purely diagnostic; it converts today's ambiguity into a leg-level flag, which is the same principle as the `Running`-vs-summary guard (§S11.7.2). Recommended as the minimum |
| **(d) do nothing** | keep the two-check precondition as the only guard (§S11.8) | acceptable only while a human reads the driver's lines; it is what made the 217 correct so far |

**Note for whoever takes this:** options (a)–(d) are *mitigations*. The underlying faults —
`Ready` never firing and the listener not binding — are the host state of §S11.1/§S11.3, whose
fix is the user-action `todo_3917fe72dae3` (restart the agent inside the current GUI session).
A driver change should not be used to paper over a measurement environment that is this
degraded, which is why (c) — make the failure visible — is the one I would take first.

**Decided (supervisor, 2026-10-17) and landed in §S12.2:** **(c) is required**, **(a) is
authorised as exactly one bounded retry**, **(b) refused** (`listener_up` was already `true`
while the children still failed, so its value is unproven and (a) subsumes it), **(d) refused
as a policy** ("acceptable only while a human reads the lines" is the property the ruling
wants removed). Neither (c) nor (a) may be used to claim the environment is healthy.

## S11.10 Segment 14: both checks fail again (4th and 3rd observation), the listener bind is measured at ~25 s, and the mechanism is narrowed — still not established

Segment 14's item (1) is a supervisor decision and **no ruling exists** in the ledger (checked:
the only open item for this unit is the hand-off/scope question itself, and the commits touching
this file since Segment 13 are the record's own). No option was landed. Item (2)'s precondition
was run and **both checks fail again**. **No measurement was taken and no ledger number changed**
— the artifact stays `/tmp/tauri-shell-lcov-c10union.info` (217) and the pinned healthy-session
figure stays §S8.3's **150**.

### S11.10.1 The two checks, 4th and 3rd observation

Run as §S11.9.1 prescribes — the already-built binary directly, **without** `ROOT_ENV` (a bare
run is a primary; setting it makes the process a companion that skips the notification):

```
CHECK A: gui_main_thread_drive: driver completed without "embedded Agent ready"; listener_up=true elapsed=17.0s
CHECK B: gui_main_thread_drive: --re-pair --re-pair-reply child exited exit status: 0 after 17.08s
         gui_main_thread_drive: bare --re-pair child exited exit status: 0 after 17.14s
         gui_main_thread_drive: no reply file at …/futureos-gui-drive-reply-16196.json within 6s
                                 (the callback's Reply arm did not finish)
```

Four observations of Check A (`without`) and three of Check B (children on the 17.1 s companion
path, no reply file). Host state is unchanged since §S11.1: `WindowServer`/`loginwindow` up
2:58:58 (the same 19:11:51 UTC restart) and the agent still the pre-restart pid 16714 from
10:18:29 UTC.

### S11.10.2 New measurement: the plugin's listener bind takes ~25 s in this session

`lsof -U` against a live primary, sampled while it ran:

| t | primary alive | `_si.sock` listeners held by the primary's pid |
|---|---|---|
| 5 s | yes | **none** |
| 15 s | yes | **none** |
| 25 s | yes | **its socket** |
| 35 s | yes | its socket |

So the plugin's `listen_for_other_instances` task (spawned during `Builder::build`) does not reach
`UnixListener::bind` for roughly 20–25 s here — the same stall that keeps `RunEvent::Ready` from
firing (§S11.1) and that made the §S11.9.4 isolation primary miss its 30 s listener budget. It is
a third quantified symptom of one degraded session, and it is the reason the driver's own
`listener_up` gate behaves erratically: on some runs its probe sees a socket quickly, on others
it waits out the budget (the Segment-13 isolation primary reported `elapsed=31.7s` with
`listener_up=false`).

**A second-instance test (with a stated confound).** A primary was left to settle, then a second
instance of the same binary was started with the primary's own `HOME`/`FUTURE_HOME`:
it ran **68.62 s** and printed **3** markers — i.e. it claimed singletonship, ran its own
watchdog and even spawned its own two notification children — instead of exiting in ~0.1 s inside
the plugin. **Confound, stated rather than buried:** the socket I verified as `ACCEPTING` before
spawning it was chosen as the newest `_si.sock` in `/tmp`, and another process does hold one —
a **13-hour-old `/opt/homebrew/bin/future-desktop` (pid 16756**, started before the session
restart, and *not* this unit's process) — so that check may not have been the primary's socket at
all. The test therefore establishes only that this second instance did not hand off; it does **not**
establish a path mismatch, and this record claims no such thing. Getting the mechanism would need
the plugin's own `tracing::debug!` output or a print of the computed socket path from inside a
run — both outside this unit's write set.

### S11.10.3 What I recommend, unchanged and now with more support

The four options are §S11.9.5's, and this segment's evidence strengthens the case for **(c) fail
loudly**: the environment can now be shown to withhold `Ready`, to bind the plugin listener ~25 s
late, and to leave the child's notify failing while the parent's own probe reports success — three
ways for a measurement to look plausible and be wrong. A driver that prints
`gui_main_thread_drive: hand-off FAILED` (and, ideally, the socket path it computed) turns all
three into leg-level flags instead of silently reading ~226 where 217 is the honest number. It
remains a decision for the supervisor, because it is a code change to this unit's file.

**Housekeeping:** the driver processes from these probes were all ended; no isolated root, reply
file or scratch file of this unit remains, and the `future-desktop` process mentioned above was
left untouched.

**Correction (found on the final sweep, after this segment's first housekeeping claim).** Two
isolated roots *did* leak during Segment 14 — `futureos-gui-drive-16414` (4 KB) and
`futureos-gui-drive-24735` (632 KB), 636 KB total — both from **this segment's own diagnostic
probes**, where I ended the primary with `SIGKILL` to keep the probe short: a `SIGKILL` cannot
run the driver's `finish()` cleanup, so the root survives. That is a property of how I killed
them, not of the driver's normal path (every ordinary run in Segments 8–13 removed its root;
§S8.1 records the one leak that was a real driver bug and was fixed). Both directories were
deleted on the spot; the sweep now shows **0** roots and **0** stray reply files. The lesson for
whoever runs these probes next: send `SIGTERM` (or let the run finish) rather than `SIGKILL`, or
remove the root by hand afterwards.

## S11.11 Segment 15: the precondition, once — Check A still fails (`without`, 5th), Check B PASSES for the first time since Segment 10

Terminal-if-blocked segment: one precondition run, then stop. Run 22:22:08 → 22:22:51 UTC, load
31.75, already-built binary, no `ROOT_ENV`; the run exited by itself and removed its root
(**0** leaked).

```
CHECK A  FAIL  gui_main_thread_drive: driver completed without "embedded Agent ready"; listener_up=true elapsed=42.9s
CHECK B  PASS  gui_main_thread_drive: bare --re-pair child exited exit status: 0 after 2.24s
               gui_main_thread_drive: --re-pair --re-pair-reply child exited exit status: 0 after 2.24s
               gui_main_thread_drive: the forwarded re-pair was answered on …/futureos-gui-drive-reply-25547.json:
                                      {"error": "the remote start ended in Failed (AccountAuthorization)"}
```

**Check B's reversal is the new evidence, and it is honest to state it plainly:** the hand-off
was failing in three consecutive observations and now succeeds, with the same attribution the
originally-good Segment-10 run had (Reply arm executed, `AccountAuthorization` answer written).
So the §S11.8–§S11.10 hand-off failure is **intermittent — load-dependent, not a permanent break
or an unestablished path mismatch** — and the "a measurement would read ~226" risk is a risk, not
a certainty. (The 2.24 s child latency versus 0.16 s on the idle host, and this run's 42.9 s
total versus 17 s, are the same load effect.) Check A, which is worth 75 of the lines, still
fails in a fifth consecutive observation, and host state is unchanged: `WindowServer` up 3:10:08
(the same 19:11:51 UTC restart), the agent still the pre-restart pid 16714, and `launchctl print
gui/501` still error 125.

**No measurement taken, no ledger number changed.** The artifact stays
`/tmp/tauri-shell-lcov-c10union.info` (217), the pinned healthy-session figure stays §S8.3's
**150**, and the class split in §S10.4 is untouched. Per this segment's instruction the todo
closes with **no successor** — six consecutive segments have reported the same blocker, and the
fix is the host action `todo_3917fe72dae3` (restart the agent inside the current GUI session),
not another shell segment. Two asks remain with the supervisor and are recorded here so they are
not lost with the todo: the **`coordinator.rs` write-set grant** for `desktop_shutdown.rs`
256–258, and the **hand-off mitigation decision** (§S11.9.5; option (c), fail loudly, recommended).

## S12 Segment 16: the supervisor's rulings landed — 256–258 have their granted test, the driver fails loudly with one retry, and the precondition still stops the measurement

This segment did exactly two pieces of work and then ran the precondition once. The rulings are
§S11.6 ask 1 (the `coordinator.rs` grant) and §S11.9.5 ask 2 ((c) fail loudly + (a) one bounded
retry), read together with the supervisor's amendment after TAU-C15 (the hand-off is
**intermittent**, and the two checks must be reported **separately**).

**No number changed.** Check A answered `without` (the 6th consecutive observation), so by the
ruling the segment stops there: the artifact stays `/tmp/tauri-shell-lcov-c10union.info` (217),
the pinned healthy-session figure stays §S8.3's **150**, and §S10.4's class split is untouched.

### S12.1 Ask 1 — the granted test for `desktop_shutdown.rs` 256–258

**Grant applied as written: test-only, in `desktop/src-tauri/src/desktop_shutdown/coordinator.rs`.
No production line changed** — not `TOTAL_BUDGET`, not `DISCONNECT_NOTICE_TIMEOUT`, and the
`pub(super)` change was not needed and not applied.

| item | value |
|---|---|
| test | `desktop_shutdown::coordinator::tests::a_budget_below_the_disconnect_notice_cap_expires_the_remote_publish` |
| location | `desktop/src-tauri/src/desktop_shutdown/coordinator.rs:379` |
| budget | `Duration::from_millis(100)` — strictly below the 750 ms cap |
| fixture | `remote::test_support::install_fake_state` (the shell's supported route; `RemoteState`/`SUPERVISOR` are `pub(in crate::remote)`) |
| result | passes, 10/10 in module-parallel runs; module 9/9 green |

**Why the arm is unreachable under the production budget** (the mechanism the row needs): the
arm's bound is `operation.remaining(Duration::from_secs(2))` = 2 s, while the future it bounds —
`remote::stop_gracefully` → `send_mobile_disconnect_notice` — is itself capped at
`DISCONNECT_NOTICE_TIMEOUT = 750 ms`. The outer timeout therefore always observes a *completed*
future, whatever the broker does. `begin_with_budget` is the only knob below that cap, which is
why the grant was needed at all.

**How the test makes the arm fire — and the honest statement of what is artificial:** the fake
bridge's tasks (broker accept loop, `async_nats` connection) live on a runtime the test creates,
and the test does not poll it again until the stop has finished. `publish` queues, `flush` waits
for a reply nobody reads, and the 100 ms budget is gone first. That is the arm's real condition
(a connection that is not making progress) made deterministic, instead of depending on a real
broker being slow. It is *not* evidence that a real slow broker behaves this way; it is evidence
that the arm's body executes when the budget is short.

**Evidence that 256–258 execute, and its limit.** The test asserts that the stop's published
reason contains exactly `"Remote shutdown deadline exceeded"`. That string exists at one place
in the crate — `desktop_shutdown.rs:257` — it is reachable only after the `is_err()` test at
255, and 258 (`crate::remote::stop()`) follows it unconditionally. So all three lines run.
**No `DA` reading is claimed.** `cargo llvm-cov` in this shell resolves the Homebrew toolchain
(`/opt/homebrew/bin/cargo`, rustc 1.96.0), which has no `llvm-tools-preview`; producing a
`DA` needs the rustup 1.97.0 toolchain and therefore a full instrumented rebuild of the tauri
dependency graph, which was not spent before the precondition (see §S12.3: the checks are
load-sensitive, and §S11.11 measured a 31.75 load effect on the same driver). **Next check when a
measurement window opens: `covlines.py detail` on lines 250–262 of `desktop_shutdown.rs`.** The
row's status is therefore **CLOSED-with-test, `DA` pending**, not waived and not ambiguous for a
fourth segment.

### S12.2 Ask 2 — (c) fail loudly, and (a) exactly one retry

Both landed in `desktop/src-tauri/tests/gui_main_thread_drive.rs`. The notification step is now a
function, `notification_round` (`:263`), called by `notify_self` (`:177`) at most twice.

The round returns `(answered, companion_path)`, where a child that outlives
`COMPANION_LATENCY = 5 s` (`:123`) is on the companion path: a hand-off exits in ~0.1–2.3 s, a
child that missed the listener becomes an app of its own and runs `READY_BUDGET + SETTLE`
(~17 s). The lines a leg must now be read for:

| when | line (`tests/gui_main_thread_drive.rs`) |
|---|---|
| reply on the first spawn | `gui_main_thread_drive: hand-off ok, first-try (the first spawn produced the reply)` — `:205` |
| first round failed | `gui_main_thread_drive: hand-off FAILED (<companion path\|children exited on the hand-off path but left no reply>; no reply within 6s); socket=/tmp/…_si.sock` — `:213` |
| listener not accepting | `gui_main_thread_drive: hand-off NOT RETRIED (nothing is accepting on …)` — `:230` |
| retry being made | `gui_main_thread_drive: retrying the notification once (spawn #2)` — `:235` |
| reply on the retry | `gui_main_thread_drive: hand-off ok, retried (the second spawn produced the reply; the first round was degraded)` — `:239` |
| retry also failed (terminal) | `gui_main_thread_drive: hand-off FAILED after one retry (…); socket=…` — `:245` |

The retry is bounded to **one**, is only taken while `listener_is_up(socket)` still answers, and
cannot mask a good hand-off because the branch is reached only when a whole round produced no
reply. The socket printed is the one the driver computed the way the plugin does
(`/tmp/<identifier>_si.sock`, with `gui_identifier`'s data-root suffix), so the §S11.10.2
confound (a 13-hour-old peer's socket) is now checkable from a log line instead of unprovable.

**Evidence — a natural passing run** (§S12.3's precondition run, first line of the pair below is
the winning case):

```
gui_main_thread_drive: bare --re-pair child exited exit status: 0 after 1.90s
gui_main_thread_drive: --re-pair --re-pair-reply child exited exit status: 0 after 1.90s
gui_main_thread_drive: the forwarded re-pair was answered on …/futureos-gui-drive-reply-11523.json: {
  "error": "the remote start ended in Failed (AccountAuthorization)"
}
gui_main_thread_drive: hand-off ok, first-try (the first spawn produced the reply)
```

**Evidence — the failure path, from a fault-injected run (labelled as such).** A passing host
cannot be made to produce the FAILED line, so the failure was injected from *outside* the binary:
the driver was launched with `TMPDIR` pointing at a writable directory and the reply path
`<TMPDIR>/futureos-gui-drive-reply-<pid>.json` was pre-created **as a directory**, so the
callback's reply write fails (`Is a directory (os error 21)` in the app log) while the children
still hand off normally. This proves the (c) and (a) code paths and nothing about the host —
the children are fast (1.83/1.84 s), which is why the reason reads "hand-off path but left no
reply" rather than "companion path". Log: `/tmp/tau-c16-fault.log`.

```
gui_main_thread_drive: bare --re-pair child exited exit status: 0 after 1.83s
gui_main_thread_drive: --re-pair --re-pair-reply child exited exit status: 0 after 1.83s
gui_main_thread_drive: no reply file at /tmp/tau-c16-fault/futureos-gui-drive-reply-29399.json within 6s (the callback's Reply arm did not finish)
gui_main_thread_drive: hand-off FAILED (children exited on the hand-off path but left no reply; no reply within 6s); socket=/tmp/com_futureos_desktop_home734ccd70c20c0a0a_si.sock
gui_main_thread_drive: retrying the notification once (spawn #2)
…… (second round, same two shapes) ……
gui_main_thread_drive: hand-off FAILED after one retry (children exited on the hand-off path but left no reply; no reply within 6s); socket=/tmp/com_futureos_desktop_home734ccd70c20c0a0a_si.sock
```

(Same shape as the first injection run, which is where the code was first shown to work; the
injected `TMPDIR` tree was deleted on the spot, and the run's own isolated root was removed by
the driver's `finish()`.)

### S12.3 The precondition, reported as two checks (the amendment's requirement)

Already-built driver binary, run directly (§S11.9.1's method), 04:15:45 → 04:16:07 UTC, load
average 3.88, log `/tmp/tau-c16-pre.log`, exit 0, **0 leaked roots and 0 stray reply files**
after the run.

```
CHECK A  FAIL  gui_main_thread_drive: driver completed without "embedded Agent ready"; listener_up=true elapsed=21.0s
CHECK B  PASS  bare --re-pair child 1.90s · --re-pair --re-pair-reply child 1.90s
               the forwarded re-pair was answered on … reply-11523.json: {"error": "…Failed (AccountAuthorization)"}
               hand-off ok, first-try
```

* **Check A (`Ready`): `without` — 6th consecutive observation.** It gates the window arms and
  the setup-hook coverage, i.e. the 75 host-state-blocked lines of §S11.4.
* **Check B (hand-off): PASS, first-try — 2nd consecutive.** It gates the **nine callback
  lines**. §S11.11 predicted this could pass again; this confirms the hand-off is intermittent
  rather than broken, and this run's reply content is the same `AccountAuthorization` answer as
  the Segment-10 and Segment-15 good runs, so the Reply arm genuinely executed.

Host state is unchanged: `WindowServer`/`loginwindow` pid 58900/58905 up **9:05:22** from the same
19:11:51 UTC session restart, `launchctl print gui/501` still **error 125**, and the only Desktop
process is the installed `/opt/homebrew/bin/future-desktop` (pid 50626, started 02:51:42 UTC) —
this worktree's driver runs its own app in its own isolated root, as designed. A Desktop exists
inside the current GUI session; this shell still cannot reach that domain, and `Ready` still does
not fire, so `todo_3917fe72dae3` (restart the agent inside the current GUI session) remains the
fix. **Neither (c) nor (a) is used here to claim the environment is healthy.**

### S12.4 Two collateral findings from the checks (both outside the 217)

1. **A test-only flake in the granted file, found and fixed.** `coordinator.rs`'s
   `a_success_after_the_deadline_is_rewritten_into_a_failure` was **9/10 red** when the module
   ran in parallel and green with `--test-threads=1`. Mechanism: it asserts on the
   process-global `STOP_FAILURE`, while every neighbour that creates or drops a `HomeGuard`
   clears that global (`HomeGuard` → `runtime::cancel_test_tasks` →
   `desktop_shutdown::reset_for_test`), and that test took no guard. My own test is **not**
   required to trigger it (measured with `--skip`), and whether it was red before this segment's
   edits is not established — but the mechanism is measured and the fix is test-only: the
   fixture now holds `HomeGuard::new("stop-late-success")` (`:499`). Module is **10/10 green**
   afterwards. This belongs in `FLAKES.md` (not this unit's write set).
2. **Lint: one pre-existing clippy error in a production line of the granted file.**
   `cargo clippy --all-targets -- -D warnings` fails at `coordinator.rs:249`
   (`nonminimal_bool`, suggests `is_none_or`) in `begin_quit`. I did **not** touch it: the grant
   is test-only and that is a non-`cfg(test)` line. With that one lint allowed
   (`-A clippy::nonminimal_bool`) the whole crate — every target, including this driver — is
   warning-free, so nothing this segment added is lint-dirty. **This needs the owner's decision
   before a CI leg sees it.** (Observed with Homebrew clippy 1.96.0; the pinned CI toolchain is
   1.97.0, where the lint also exists.)

**Test evidence for this segment's files:** `cargo test --lib -- desktop_shutdown::coordinator::tests`
→ 9/9 (and 10/10 runs of the module green); the full `cargo test --lib` in parallel gave
**2090 passed / 6 failed**, all six in peer modules (`terminal::session`, `terminal::server`,
`terminal::pty`, one `remote::commands::bridge_tests`) and **all six green when each is run
serially** — the measurement recipe runs `--test-threads=1` (§1), so this does not touch the
leg. `cargo fmt --check` clean. Driver target compiles and runs (above).

---

## S13 Segment 17 (TAU-SHELL-LAND): the review's four families landed, and the measured figure is **115**

This segment did what `todo_f65783eb7365` asks: land REV-TAURI-SHELL's fallen families as ordinary
tests, re-measure both flavours, and publish a **measured** number instead of the two estimates
(107 and 127) that were on the table. No production behaviour changed; every change below is test
code or test fixtures, plus this record.

### S13.1 What landed, and where

| # | family (review group) | landed as | rows it was meant to move |
|---|---|---|---|
| 1 | `macos_power.rs` observers (S10.20) | `macos_power.rs` `#[cfg(test)] mod tests::posting_the_workspace_notifications_runs_the_observer_bodies` — posts the three `NSWorkspace*Notification` names to `NSWorkspace::sharedWorkspace().notificationCenter()`, where the blocks were registered, so they run synchronously on the posting thread | 10 |
| 2 | `menu.rs` builder (S10.11) | `tests/gui_main_thread_drive/body.rs::build_the_menu` (`harness = false`, i.e. the main thread) + the existing `#[cfg(feature = "gui")] #[doc(hidden)] pub use menu::build_macos_menu;` re-export in `lib.rs` that makes the builder reachable from an integration target | 46 |
| 3 | launcher + bin argv (S10.9) | `tests/re_pair_entry_points.rs` — 4 tests: `forward_re_pair` from an integration test, the bin refusing `--re-pair` without a terminal, the pty launcher forwarding to a real Desktop that answers, and the pty launcher reporting a hand-off nobody answered (this unit's own file lock held) | 27 |
| 4 | `RunEvent::ExitRequested` (S10.7) | `lib.rs` `mod gui` → `startup_paths_tests::an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm` — the mock runtime's own loop builds the event and hands a real `ExitRequestApi` to the callback | 6 |
| 5 | review correction 4 (`lib.rs` 1906–1907) | inside `print_re_pair_notice_writes_the_launcher_notice`: the test double's accepting `write`/`flush` halves are now called directly | 2 |

Every one of these is a test that asserts the behaviour the line implements: the observers run
bodies that publish remote state, the menu is asserted to have four top-level submenus, the pty
tests assert the printed invitation notice (or the refusal text) and the exit status, and the
`ExitRequested` test asserts that the Proceed arm **published a stop named `quit`** — not merely
that the closure ran.

### S13.2 The measurement — **115**, and what it is

Recipe (§S4.1/§S10.2, unchanged): `cargo llvm-cov` **per flavour**, rendered immediately after its
own leg, `clean` **between** legs, then union. Private target dir, isolated `HOME`,
`ulimit -n 10240`, `-j 4`, `--test-threads=1` immediately after the single `--`.

```
== gui leg @ 22:05:24 ==    cargo llvm-cov --lcov --no-fail-fast -j 4 --output-path /tmp/tauri-shell-land/gui.info -- --test-threads=1
GUI_EXIT=0                  test result: ok. 2142 passed; 0 failed; 14 ignored; 0 measured
== clean @ 22:14:41 ==      CLEAN=0
headless leg               /tmp/tauri-shell-land/headless-prev.info (leg of the same recipe, HEADLESS_EXIT=0,
                           1717 passed / 0 failed)
UNION_EXIT=0   ALIVE_EXIT=0   LEGCHECK_EXIT=0
SF counts: gui=170 headless=138 union=172
```

Load at the gui leg: **1-min 4.10, 5-min 2.97 on 32 CPUs** (before the leg); 1-min 64 at the end,
foreign (`life-journal-server` indexing), i.e. the suite was measured under a rising but not
saturated load (§S11.11 measured a 31.75-load effect on the same driver, which is why every wait in
these tests is bounded).

**Why the headless report is one leg older than the gui report, and why that is still a valid
union.** The headless leg was rendered at 21:44 from the source before this segment's last two
edits; the gui leg at 22:17 from the final source. The edits are inside `#[cfg(feature = "gui")]
mod gui` (and `tests/`, which no flavour's library build compiles), and the headless library does
not compile `mod gui` at all: its `lib.rs` report carries **57** `DA` records, the highest being
line **174** — i.e. every line it reports is above `mod gui`'s first line (196). Line numbers in
the headless report are therefore unaffected by the edits, and the union is sound. (Checked, not
assumed: `python3 -c` over `headless-prev.info` → `max DA line = 174`.)

| scope | measured | covered | uncovered |
|---|---|---|---|
| `src/lib.rs` | 1281 | 1206 | **75** |
| `src/desktop_shutdown.rs` | 452 | 439 | **13** |
| `src/desktop_shutdown/coordinator.rs` | 369 | 361 | **8** |
| `src/runtime.rs` | 216 | 212 | **4** |
| `src/future_login.rs` | 866 | 863 | **3** |
| `src/instance.rs` | 153 | 148 | **5** |
| `src/main.rs` | 88 | 86 | **2** |
| `src/menu.rs` | 48 | 46 | **2** |
| `src/launch.rs` | 562 | 560 | **2** |
| `src/bin/futureos-headless.rs` | 51 | 50 | **1** |
| `src/macos_power.rs` | 49 | 49 | **0** |
| `src/host_environment.rs` | 43 | 43 | **0** |
| **total (twelve scopes)** | **4178** | **4063** | **115** |

**The number is 115.** It is not 107 and not 127, and the two are not rival readings of the same
ledger:

* **115** is what this session measured, on this source, with `Ready` still absent (seven
  consecutive observations now — §S12.3 was the sixth).
* **90** is the projection for a `Ready`-delivering session under the same source: 25 of the 115
  are the setup-hook/window family (§S13.5 S13.1 + S13.2j), 18 of which resolve by unique source
  text to a line the pinned c8 artifact covers, the rest being the headers and braces of those same
  blocks. **That projection is labelled a projection** — it is arithmetic on a class, not a
  measurement, and it is deliberately not published as "the new pinned figure": §S8.3's **150**
  stays the pinned healthy-session number until a session that delivers `Ready` measures the source.
* **107** was the review-side arithmetic (`150 − 43`). It under-counts the closure because this
  ticket closed rows the review did not count as closing (its own four families were worth 88
  measured rows, not 43 — see §S13.3 — and the shutdown/attribution families it left alone moved
  too), and it is a healthy-session figure, which this session cannot produce.
* **127** was the expectation that only `macos_power` (10) and `launch.rs` (13) would close inside
  the 150. Measured, `menu.rs` closed **46** and `main.rs` closed **13 of 14** as well, so 127 is
  refuted by the artifact, not by argument.

Two earlier runs of the same recipe bracket this one, and the differences are **exactly** the rows
the two fixes closed — not run-to-run noise: **123** (union at 21:44, before the `ExitRequested`
isolation fix) → **117** (22:00) closed the six test-body continuation lines the fresh-process
branch replaced (`lib.rs` 2834, 2835, 2872–2875 in that run's numbering) and opened none; **117 →
115** closed the two accepting halves of the `Failed` double (`lib.rs` 1949, 1950). Set-diffed over
the three reports: no other row changed in any direction.

### S13.3 The review's 90 FELL rows, re-measured one by one

Per row, the review's line was resolved to the current line by source text (the review's revision
is `3065c8cb7`, 55 commits back; six rows changed text when the `--re-pair` QR change (#1162/#1163)
merged, and those are resolved by hand and marked †).

| review group | rows | now covered | not covered |
|---|---|---|---|
| S10.20 `macos_power` | 10 | **10** | — |
| S10.11 `menu.rs` | 46 | **46** | — |
| S10.9 `launch.rs` 330–347 | 13 | **13**† | — |
| S10.9 `main.rs` | 14 | **13**† | `main.rs:51` |
| S10.7 `ExitRequested` | 6 | **6** | — |
| S10.2 `lib.rs:993` (`APP_HANDLE.set`) | 1 | 0 | `lib.rs:1040` |
| **total** | **90** | **88** | **2** |

The two that resisted, with why:

* **`main.rs:51`** is the `}` that closes the `if options.re_pair {` block the pty launcher
  executes: its neighbours (38–50, 52–53) are covered **and were covered in the review's own union
  (its 51 = 1)**, so this is region attribution, not behaviour — the same shape as `menu.rs:41/48`
  and `desktop_shutdown.rs:460`. Class: `attribution-artifact`.
* **`lib.rs:1040`** (`let _ = APP_HANDLE.set(app.handle().clone());`) is the first statement of the
  `setup` hook. The review counted it FELL because its scratch probe started a real GUI process
  that ran `setup`; this session's driver still never reaches `setup` (§S12.3), so it stays with the
  host-state family. It is not evidence against the review — it is the same `Ready` boundary, and
  it returns with the 24 others when `Ready` does.

† `launch.rs` 330–347 and `main.rs` 38/41–43 changed text (`forward_re_pair(no_qr)`,
`invitation_notice(&reply, !options.no_qr)`, `print_to_terminal`). Their successors are covered; the
pty tests are what executes them (below).

### S13.4 The six mechanism corrections (§4 of the review), as the record now states them

1. **`menu.rs` is main-thread, not host-state.** Measured: **46 of 48** rows covered by a
   `harness = false` target on this host with no `Ready` (driver line
   `gui_main_thread_drive: menu built on the main thread (4 top-level submenus)`). The remaining
   two are the `)?)` continuations (attribution). §S8.3's "only the setup hook calls it" was wrong
   about the *requirement*: the requirement is the main thread, and a test target is that thread.
2. **`needs-window` (3 rows) is `host-state-blocked`, not a design boundary.** The three callback
   window calls are now `activate_main_window`'s lines (`lib.rs` 212–217, 244–246): they stay
   uncovered here because `get_webview_window("main")` answers `None` in this session, and the
   pinned c8 artifact has them covered. Same class as the other 22 setup-hook rows, not a waiver.
3. **`main.rs:9` is `platform-unmeasured`.** The `--linux-sandbox-helper` *success* arm: on macOS
   the same argv fails at the argument parse, which is what the refusal test observes. Measured: 1
   row, uncovered, and it is the only `main.rs` row besides the brace.
4. **S10.12's reason is `cfg(test)`, and `cfg(test)` is the wrong boundary.** It is false for an
   **integration** build, so `desktop_shutdown.rs` 221–223/267–274 are compiled into every
   `tests/` binary — the real boundary is that `perform_stop` is private and only a quit reaches
   it. Measured: all 11 rows still uncovered, and the reason is sharper than "not compiled": the
   driver ends its process with `libc::exit` (so the profile is written), which means
   `RunEvent::Exit` never runs and `perform_stop` is never entered. A spawned GUI that exits
   through `RunEvent::Exit` would execute them; that is a different fixture, not a waiver change.
5. **`lib.rs:1727` (now 1770) is inside a test body.** The burst producer's early `return` when
   `send` fails. Uncovered because the fixture keeps its receiver alive until after the producer
   is joined — the property the same test asserts (coalescing under a live stream). It stays
   `unreachable-by-construction` **inside a test**, and it is not a production gap.
6. **`ExitRequestApi` is also built by the mock loop.** Measured: the `ExitRequested` Proceed arm
   runs from `tauri::test::mock_app()`'s own loop (`tauri-2.11.5/src/test/mock_runtime.rs`), the
   real API in hand, and publishes the `quit` stop. 6 of 11 rows closed; the `Confirm` arms
   (`lib.rs` 1316–1320) still need active work or an open dialog.

### S13.5 The new per-line ledger — 115 rows, current line numbers

**Superseded by §S13.10:** this is Segment 17's 115-row ledger on that revision. Six of its rows
(212, 214, 215, 217, 244, 246) were closed by the mock-window tests; §S13.10 carries the
re-measurement, the 109-row ledger and the index the pairing now points at.

Every row below is `DA:0` in `/tmp/tauri-shell-land/union.info`. The class is the campaign's
policy class (`docs/testing/waiver-ledger.md`) plus `host-state-blocked` for rows the pinned
healthy artifact covers; a closing brace is grouped with the block it closes (the convention
§S8.3 used).

| group | rows | class | mechanism |
|---|---|---|---|
| S13.1 | 24 | `host-state-blocked` | the `setup` hook and a main window: 18 of the 25 rows of this family resolve to a line the pinned c8 artifact covers (215, 261-267, 526-529, 1002, 1005, 1040, 1042, 1052, 1058); the rest are the block headers, the `Ok(())` and the braces of those same blocks — `lib.rs` 212,215,244,261,262,263,264,265,266,267,526,527,528,529,530,1002,1005,1040,1042,1052,1058,1078,1079,1080 |
| S13.1b | 6 | `unreachable-in-this-environment` | the failure bodies (and braces) of `unminimize`/`show`/`set_focus`: the calls succeed wherever the setup runs, so c8 leaves the same body uncovered (its 1033 = 0 under a covered 1032) — `lib.rs` 213,214,216,217,245,246 |
| S13.2a | 2 | `unreachable-in-this-environment` | `APP_HANDLE` is `OnceLock<AppHandle<Wry>>`; a mock or test app never publishes one — `lib.rs` 863,894 |
| S13.2b | 3 | `unreachable-in-this-environment` | `gui::run()` is the mobile entry point; no desktop caller exists — `lib.rs` 940,941,942 |
| S13.2c | 11 | `unreachable-in-this-environment` | a real AppKit menu click posts `on_menu_event`; no test synthesizes one — `lib.rs` 979,980,981,982,983,984,988,989,990,992,997 |
| S13.2d | 14 | `unreachable-in-this-environment` | the arms of a real window-close request (entry lines covered, arms need a close and an active-work decision) — `lib.rs` 1003,1004,1007,1009,1010,1011,1012,1014,1015,1016,1017,1018,1021,1022 |
| S13.2e | 4 | `unreachable-in-this-environment` | failure arms of real calls that succeed here (terminal bind, agent supervision) — `lib.rs` 1268,1293,1294,1297 |
| S13.2f | 5 | `unreachable-in-this-environment` | the `ExitRequested` Confirm arm: needs active work or an open dialog — `lib.rs` 1316,1317,1318,1319,1320 |
| S13.2h | 2 | `attribution-artifact` | the split `if let Err(error) =` / `{` of the multi-line menu-setup condition; c8 shows the same shape (old 1004 = 0 under a covered 1005) — `lib.rs` 1051,1053 |
| S13.2i | 2 | `unreachable-in-this-environment` | the menu-setup failure body; `build_macos_menu` succeeds — `lib.rs` 1054,1055 |
| S13.2j | 1 | `host-state-blocked` | run by the `setup` hook, immediately before `size_main_window_to_screen` (c8: covered) — `lib.rs` 1057 |
| S13.2g | 1 | `unreachable-by-construction` | inside a test: the burst producer's early return, which the fixture's live receiver forbids — `lib.rs` 1770 |
| S13.3a | 1 | `platform-unmeasured` | the `--linux-sandbox-helper` **success** arm — `main.rs:9` |
| S13.3b | 1 | `attribution-artifact` | the `}` closing the re-pair block the pty launcher executes; neighbours 38-50, 52-53 covered — `main.rs:51` |
| S13.3c | 2 | `attribution-artifact` | the two `)?)` continuations; neighbours covered, builder runs — `menu.rs:41,48` |
| S13.3d | 2 | `unreachable-in-this-environment` | `current_exe()` failing: a process cannot fail to locate its own executable — `launch.rs:486,487` |
| S13.3e | 5 | `unreachable-in-this-environment` | the flock re-acquire retry/panic arms; the lock is released and re-acquired first try, and the fixture has no failure injection — `instance.rs:164-168` |
| S13.3f | 4 | `unreachable-by-construction` | inside a test: the frozen registry's closures, which the test asserts never run — `runtime.rs:248,249,251,252` |
| S13.3g | 11 | `unreachable-in-this-environment` | the terminal-manager stages of `perform_stop`: compiled in integration builds, but only a clean `RunEvent::Exit` reaches the private `perform_stop`, and the driver exits with `libc::exit` — `desktop_shutdown.rs:221-223,267-274` |
| S13.3h | 1 | `attribution-artifact` | a block-end brace inside a test; neighbours 456-459 covered — `desktop_shutdown.rs:460` |
| S13.3i | 1 | `unreachable-by-construction` | inside a test: the brace of a `std::future::pending` future the fixture asserts never completes — `desktop_shutdown.rs:501` |
| S13.3j | 1 | `unreachable-in-this-environment` | the progress emit needs `APP_HANDLE`, i.e. a real `AppHandle<Wry>` — `coordinator.rs:50` |
| S13.3k | 7 | `unreachable-in-this-environment` | OS thread-creation refusal (watchdog and worker spawn); needs thread/`RLIMIT` exhaustion — `coordinator.rs:165,166,183,184,185,186,187` |
| S13.3l | 3 | `unreachable-in-this-environment` | a poll endpoint returning a retryable body error, and a `reqwest` error with an empty cause chain — `future_login.rs:354,355,482` |
| S13.3m | 1 | `platform-unmeasured` | the Linux-only helper success arm — `bin/futureos-headless.rs:11` |
| **total** | **115** | | |

Class totals: **`host-state-blocked` 25**, **`unreachable-in-this-environment` 76**,
**`attribution-artifact` 6**, **`unreachable-by-construction` 6**, **`platform-unmeasured` 2** —
sum 115, and a cross-check over the report
(`/tmp/classify.py /tmp/tauri-shell-land/union.info`, kept at `/tmp/ledger-classes.txt`) leaves
**no unexplained row**.


### Per-line index of this segment's residual (115 rows)

Machine-readable form of §S13.5, in the same shape as §S8.3's and §S10.4's indexes, so a
pairing can measure it: every row is `DA:0` in `/tmp/tauri-shell-land/union.info` (the gui and
headless flavours unioned, per §S13.2).

| line | group |
|---|---|
| `src/bin/futureos-headless.rs:11` | S13.3m |
| `src/desktop_shutdown.rs:221` | S13.3g |
| `src/desktop_shutdown.rs:222` | S13.3g |
| `src/desktop_shutdown.rs:223` | S13.3g |
| `src/desktop_shutdown.rs:267` | S13.3g |
| `src/desktop_shutdown.rs:268` | S13.3g |
| `src/desktop_shutdown.rs:269` | S13.3g |
| `src/desktop_shutdown.rs:270` | S13.3g |
| `src/desktop_shutdown.rs:271` | S13.3g |
| `src/desktop_shutdown.rs:272` | S13.3g |
| `src/desktop_shutdown.rs:273` | S13.3g |
| `src/desktop_shutdown.rs:274` | S13.3g |
| `src/desktop_shutdown.rs:460` | S13.3h |
| `src/desktop_shutdown.rs:501` | S13.3i |
| `src/desktop_shutdown/coordinator.rs:50` | S13.3j |
| `src/desktop_shutdown/coordinator.rs:165` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:166` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:183` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:184` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:185` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:186` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:187` | S13.3k |
| `src/future_login.rs:354` | S13.3l |
| `src/future_login.rs:355` | S13.3l |
| `src/future_login.rs:482` | S13.3l |
| `src/instance.rs:164` | S13.3e |
| `src/instance.rs:165` | S13.3e |
| `src/instance.rs:166` | S13.3e |
| `src/instance.rs:167` | S13.3e |
| `src/instance.rs:168` | S13.3e |
| `src/launch.rs:486` | S13.3d |
| `src/launch.rs:487` | S13.3d |
| `src/lib.rs:212` | S13.1 |
| `src/lib.rs:213` | S13.1b |
| `src/lib.rs:214` | S13.1b |
| `src/lib.rs:215` | S13.1 |
| `src/lib.rs:216` | S13.1b |
| `src/lib.rs:217` | S13.1b |
| `src/lib.rs:244` | S13.1 |
| `src/lib.rs:245` | S13.1b |
| `src/lib.rs:246` | S13.1b |
| `src/lib.rs:261` | S13.1 |
| `src/lib.rs:262` | S13.1 |
| `src/lib.rs:263` | S13.1 |
| `src/lib.rs:264` | S13.1 |
| `src/lib.rs:265` | S13.1 |
| `src/lib.rs:266` | S13.1 |
| `src/lib.rs:267` | S13.1 |
| `src/lib.rs:526` | S13.1 |
| `src/lib.rs:527` | S13.1 |
| `src/lib.rs:528` | S13.1 |
| `src/lib.rs:529` | S13.1 |
| `src/lib.rs:530` | S13.1 |
| `src/lib.rs:863` | S13.2a |
| `src/lib.rs:894` | S13.2a |
| `src/lib.rs:940` | S13.2b |
| `src/lib.rs:941` | S13.2b |
| `src/lib.rs:942` | S13.2b |
| `src/lib.rs:979` | S13.2c |
| `src/lib.rs:980` | S13.2c |
| `src/lib.rs:981` | S13.2c |
| `src/lib.rs:982` | S13.2c |
| `src/lib.rs:983` | S13.2c |
| `src/lib.rs:984` | S13.2c |
| `src/lib.rs:988` | S13.2c |
| `src/lib.rs:989` | S13.2c |
| `src/lib.rs:990` | S13.2c |
| `src/lib.rs:992` | S13.2c |
| `src/lib.rs:997` | S13.2c |
| `src/lib.rs:1002` | S13.1 |
| `src/lib.rs:1003` | S13.2d |
| `src/lib.rs:1004` | S13.2d |
| `src/lib.rs:1005` | S13.1 |
| `src/lib.rs:1007` | S13.2d |
| `src/lib.rs:1009` | S13.2d |
| `src/lib.rs:1010` | S13.2d |
| `src/lib.rs:1011` | S13.2d |
| `src/lib.rs:1012` | S13.2d |
| `src/lib.rs:1014` | S13.2d |
| `src/lib.rs:1015` | S13.2d |
| `src/lib.rs:1016` | S13.2d |
| `src/lib.rs:1017` | S13.2d |
| `src/lib.rs:1018` | S13.2d |
| `src/lib.rs:1021` | S13.2d |
| `src/lib.rs:1022` | S13.2d |
| `src/lib.rs:1040` | S13.1 |
| `src/lib.rs:1042` | S13.1 |
| `src/lib.rs:1051` | S13.2h |
| `src/lib.rs:1052` | S13.1 |
| `src/lib.rs:1053` | S13.2h |
| `src/lib.rs:1054` | S13.2i |
| `src/lib.rs:1055` | S13.2i |
| `src/lib.rs:1057` | S13.2j |
| `src/lib.rs:1058` | S13.1 |
| `src/lib.rs:1078` | S13.1 |
| `src/lib.rs:1079` | S13.1 |
| `src/lib.rs:1080` | S13.1 |
| `src/lib.rs:1268` | S13.2e |
| `src/lib.rs:1293` | S13.2e |
| `src/lib.rs:1294` | S13.2e |
| `src/lib.rs:1297` | S13.2e |
| `src/lib.rs:1316` | S13.2f |
| `src/lib.rs:1317` | S13.2f |
| `src/lib.rs:1318` | S13.2f |
| `src/lib.rs:1319` | S13.2f |
| `src/lib.rs:1320` | S13.2f |
| `src/lib.rs:1770` | S13.2g |
| `src/main.rs:9` | S13.3a |
| `src/main.rs:51` | S13.3b |
| `src/menu.rs:41` | S13.3c |
| `src/menu.rs:48` | S13.3c |
| `src/runtime.rs:248` | S13.3f |
| `src/runtime.rs:249` | S13.3f |
| `src/runtime.rs:251` | S13.3f |
| `src/runtime.rs:252` | S13.3f |
<!-- 115 rows -->

### S13.6 A test-isolation defect this segment found and fixed (test-only)

The `ExitRequested` test passed alone and **failed in the full gui suite** (1 failed / 2141 passed,
`the Proceed arm published a stop`). Mechanism, measured: `agent_supervisor::exit_authorized()`
reads the process-global `QUIT_CONFIRMED`, which the peer test
`agent_supervisor::tests::an_authorized_exit_proceeds_without_a_dialog` sets; libtest runs
`agent_supervisor::` before `gui::` in one process, so by the time the probe ran the exit was
already "authorized" and the two rows the test exists for (`api.prevent_exit()` and
`request_quit`) were skipped. There is no reset for that flag, and `agent_supervisor.rs` is outside
this unit's write set — so the body now re-runs itself once in a **fresh process**
(`current_exe()` with `--exact`, a `FUTUREOS_EXIT_REQUESTED_CHILD` marker to avoid recursion). The
child's profile merges like any spawned fixture's, which is the same property the pty launcher
tests rely on. Measured: `--exact …takes_the_guarded_quit_arm` → ok in 1.05 s; full gui suite →
**2142 passed, 0 failed**.

Two smaller things the same review corrected, both landed here: the `Failed` sink's accepting
`write`/`flush` halves are now called (2 rows, `lib.rs` 1906–1907 at the review's revision), and
the fresh-process branch's assertion is one line so it does not leave an uncovered continuation in
the measured region.

### S13.7 Gates (PLAN §5.1), both flavours, exit codes as run

```
cargo fmt --check --manifest-path desktop/src-tauri/Cargo.toml                         FMT_RC=0
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings CLIPPY_GUI_RC=0
cargo clippy ... --all-targets --no-default-features --features headless -- -D warnings
                                                                                      CLIPPY_HEADLESS_RC=0
```

rustc/cargo 1.97.0 (the pinned toolchain), `LLVM_COV`/`LLVM_PROFDATA` pointed at that toolchain's
own `llvm-tools` binaries (cargo-llvm-cov 0.8.7 looks for `llvm-tools-preview` and finds nothing).
Formatting was checked per file while editing and crate-wide only here, at the end of the segment,
when no other unit was mid-edit in this crate.

Test evidence, from the gui leg that produced the report:

```
test result: ok. 2142 passed; 0 failed; 14 ignored; 0 measured; 0 filtered out        (lib)
test result: ok. 2 passed                                                        (bin futureos)
gui_main_thread_drive: menu built on the main thread (4 top-level submenus)
gui_main_thread_drive: bare --re-pair child exited exit status: 0 after 0.17s
gui_main_thread_drive: --re-pair --re-pair-reply child exited exit status: 0 after 0.17s
gui_main_thread_drive: the forwarded re-pair was answered on …/futureos-gui-drive-reply-93958.json
gui_main_thread_drive: hand-off ok, first-try (the first spawn produced the reply)
gui_main_thread_drive: driver completed without "embedded Agent ready"; listener_up=true elapsed=17.4s
test result: ok. 4 passed; 0 failed                                              (re_pair_entry_points)
```

`posting_the_workspace_notifications_runs_the_observer_bodies`: ok (in the 2142).

### S13.8 Reproduction

```bash
WT=/Users/geilige/future-os/.worktrees/cov100-r2
cd $WT/desktop/src-tauri
for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME="$WT/target/test-home" CARGO_TARGET_DIR="$WT/target/cov-tauri-shell-land"
ulimit -n 10240
B=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin
export LLVM_COV=$B/llvm-cov LLVM_PROFDATA=$B/llvm-profdata   # ABSOLUTE: `~` expands after HOME moves

cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --output-path /tmp/tauri-shell-land/gui.info -- --test-threads=1
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --no-default-features --features headless \
  --output-path /tmp/tauri-shell-land/headless.info -- --test-threads=1
python3 ../../.future/cov100/lcov-union.py --out /tmp/tauri-shell-land/union.info \
  /tmp/tauri-shell-land/gui.info /tmp/tauri-shell-land/headless.info
python3 ../../.future/cov100/flavor-alive.py  /tmp/tauri-shell-land/gui.info /tmp/tauri-shell-land/headless.info
python3 ../../.future/cov100/leg-flavour-check.py /tmp/tauri-shell-land/gui.info /tmp/tauri-shell-land/headless.info
```

Two traps this segment paid for, both worth keeping: (1) `B=~/.rustup/...` is expanded **after**
`HOME` is redirected, so `llvm-profdata` is looked up under the test home and cargo-llvm-cov fails
at the merge step *after* both legs have run — write absolute paths; (2) a leg that fails at that
step renders **no** report, so `--output-path` alone is not enough: check `-s` on the file and
render with `cargo llvm-cov report` if the leg exited non-zero.

### S13.9 What this segment does not claim

* **Not a `Ready`-delivering measurement.** Check A is `without` for the seventh consecutive
  observation; the 25 host-state rows stay uncovered here and the 90 is a labelled projection.
  (After §S13.10 closed the three mock-reachable ones: 22 rows, 109 measured, 87 projected.)
* **Not a re-opening of the 96 SURVIVED or 31 UNVERIFIABLE rows.** Their verdicts stand; the two
  rows this segment moved out of correction 4 were moved by *adding the call the review named*, not
  by disputing a verdict.
* **Not a claim about the pinned figure.** §S8.3's **150** and §S10.4's **217** stay pinned as
  historical figures; this segment adds **115 measured / 90 projected** on the current source at
  its revision, and §S13.10's mock-window closure moves the current figures to **109 / 87**.
* **No production line changed** in this segment's scope except the one pre-existing
  `nonminimal_bool` fix in `coordinator.rs:249` that the previous supervisor committed with
  `-D warnings` (`!x.is_some_and(p)` → `x.is_none_or(!p)`, behaviour-equivalent for `Option`).

### S13.10 The mock-window closure — **115 → 109**

The three condition rows §S13.5 left as `host-state-blocked` because "this session's app never
creates a window" are reachable after all. `activate_main_window` (`lib.rs:206`) acts on whatever
the manager holds under the label `main`, and the **mock runtime** can hold one. Two tests in
`gui::startup_paths_tests` now build that window and call the function:

* `activate_main_window_restores_shows_and_focuses_the_main_window` builds a mock app, builds a
  webview window labelled `main`, calls `activate_main_window(app.handle())`, and asserts the
  window is still there under that label. `unminimize`, `show` and `set_focus` answer `Ok(())` in
  the mock runtime (`tauri-2.11.5/src/test/mock_runtime.rs`: `unminimize` :972, `show` :976,
  `set_focus` :1043), so the `Some` arm runs end to end instead of returning at the `None` check.
* `activate_main_window_tolerates_a_missing_main_window` exercises the `None` arm explicitly
  (already covered through the dispatched run-event tests; asserted here so it does not depend on
  which other test happens to call the function).

**Six rows closed, not three.** The three conditions (`lib.rs` 212, 215, 244) are the ones the
task named; the `}` closing each `if let Err(..)` body (214, 217, 246) fell with them, because with
the condition false control flows past the brace and llvm-cov attributes that region as executed —
the mirror image of §S13.2h's split-condition artifact, where the brace is *not* attributed. The
three error bodies (213, 216, 245) stay `DA:0`: the mock runtime never fails those calls.
`S13.1b` is therefore 6 → 3 rows, and the class split moves on both sides.

Measurement — the gui leg re-rendered on the current source and unioned with the headless leg,
which §S13.2's argument still covers (`mod gui` starts at `lib.rs:196`; the headless report's
highest `DA` line is **174**, re-checked against the current source):

```
== load before gui @ 23:21:45 ==   load averages: 4.26 3.51 3.80
CLEAN=0                  cargo llvm-cov clean, before the leg
== gui leg @ 23:21:45 == cargo llvm-cov --lcov --no-fail-fast -j 4 --output-path /tmp/tauri-shell-land/gui.info -- --test-threads=1
GUI_EXIT=0               test result: ok. 2144 passed; 0 failed; 14 ignored
UNION_EXIT=0  ALIVE_EXIT=0  LEGCHECK_EXIT=0
SF counts: gui=170 headless=138 union=172
== done @ 23:33:11 ==    load averages: 2.76 2.67 3.17
```

| scope | measured | covered | uncovered |
|---|---|---|---|
| `src/lib.rs` | 1298 | 1229 | **69** |
| `src/desktop_shutdown.rs` | 452 | 439 | **13** |
| `src/desktop_shutdown/coordinator.rs` | 369 | 361 | **8** |
| `src/runtime.rs` | 216 | 212 | **4** |
| `src/future_login.rs` | 866 | 863 | **3** |
| `src/instance.rs` | 153 | 148 | **5** |
| `src/main.rs` | 88 | 86 | **2** |
| `src/menu.rs` | 48 | 46 | **2** |
| `src/launch.rs` | 562 | 560 | **2** |
| `src/bin/futureos-headless.rs` | 51 | 50 | **1** |
| `src/macos_power.rs` | 49 | 49 | **0** |
| `src/host_environment.rs` | 43 | 43 | **0** |
| **total (twelve scopes)** | **4195** | **4086** | **109** |

`lib.rs` measured grows 1281 → 1298 because the two fixtures sit inside the measured scope and their
executed lines are now `DA` records; covered grows 1206 → 1229 (the six flipped rows plus those test
lines). Every other scope is unchanged to the line.

Class totals over the 109: **`host-state-blocked` 22** (was 25), **`unreachable-in-this-environment`
73** (was 76), **`attribution-artifact` 6**, **`unreachable-by-construction` 6**,
**`platform-unmeasured` 2** — sum 109, and
`python3 /tmp/classify.py /tmp/tauri-shell-land/union.info` prints `UNEXPLAINED: []`.

**The projection for a `Ready`-delivering session is 87, not 90.** The projection subtracts the
`host-state-blocked` class (22 now: 109 − 22). The expectation was 90 because it counted only the
three condition rows as closing; the three braces are `unreachable-in-this-environment` and they
closed too, so the arithmetic lands three lower. §S8.3's **150** still stays the pinned
healthy-session figure until a session that delivers `Ready` measures this source — the 87 is
arithmetic on a class, labelled as such, exactly like the 90 it replaces.

What this section does **not** claim: the other 22 `host-state-blocked` rows still need a session
that delivers `Ready` (the host action is `todo_3917fe72dae3`), the 76 → 73 environment rows and
the 6 + 6 + 2 rows are untouched, and `size_main_window_to_screen` 261-267 stays — the mock's
`current_monitor()` returns `None` (`mock_runtime.rs`:785), so that call site remains unreachable.
The 11 terminal-stage rows of `perform_stop` stay as §S13.3g classifies them: the gui-only block is
`cfg(not(test))` and the function is `pub(crate)`, so an integration test cannot reach it.

### Per-line index of this segment's residual (109 rows)

Machine-readable form of the ledger above, in the same shape as §S13.5's index: every row is
`DA:0` in `/tmp/tauri-shell-land/union.info` (the gui and headless flavours unioned, per §S13.2),
grouped by the class ranges §S13.5 states. Group ids are unchanged; the rows they cover are not.

| line | group |
|---|---|
| `src/bin/futureos-headless.rs:11` | S13.3m |
| `src/desktop_shutdown.rs:221` | S13.3g |
| `src/desktop_shutdown.rs:222` | S13.3g |
| `src/desktop_shutdown.rs:223` | S13.3g |
| `src/desktop_shutdown.rs:267` | S13.3g |
| `src/desktop_shutdown.rs:268` | S13.3g |
| `src/desktop_shutdown.rs:269` | S13.3g |
| `src/desktop_shutdown.rs:270` | S13.3g |
| `src/desktop_shutdown.rs:271` | S13.3g |
| `src/desktop_shutdown.rs:272` | S13.3g |
| `src/desktop_shutdown.rs:273` | S13.3g |
| `src/desktop_shutdown.rs:274` | S13.3g |
| `src/desktop_shutdown.rs:460` | S13.3h |
| `src/desktop_shutdown.rs:501` | S13.3i |
| `src/desktop_shutdown/coordinator.rs:50` | S13.3j |
| `src/desktop_shutdown/coordinator.rs:165` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:166` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:183` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:184` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:185` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:186` | S13.3k |
| `src/desktop_shutdown/coordinator.rs:187` | S13.3k |
| `src/future_login.rs:354` | S13.3l |
| `src/future_login.rs:355` | S13.3l |
| `src/future_login.rs:482` | S13.3l |
| `src/instance.rs:164` | S13.3e |
| `src/instance.rs:165` | S13.3e |
| `src/instance.rs:166` | S13.3e |
| `src/instance.rs:167` | S13.3e |
| `src/instance.rs:168` | S13.3e |
| `src/launch.rs:486` | S13.3d |
| `src/launch.rs:487` | S13.3d |
| `src/lib.rs:213` | S13.1b |
| `src/lib.rs:216` | S13.1b |
| `src/lib.rs:245` | S13.1b |
| `src/lib.rs:261` | S13.1 |
| `src/lib.rs:262` | S13.1 |
| `src/lib.rs:263` | S13.1 |
| `src/lib.rs:264` | S13.1 |
| `src/lib.rs:265` | S13.1 |
| `src/lib.rs:266` | S13.1 |
| `src/lib.rs:267` | S13.1 |
| `src/lib.rs:526` | S13.1 |
| `src/lib.rs:527` | S13.1 |
| `src/lib.rs:528` | S13.1 |
| `src/lib.rs:529` | S13.1 |
| `src/lib.rs:530` | S13.1 |
| `src/lib.rs:863` | S13.2a |
| `src/lib.rs:894` | S13.2a |
| `src/lib.rs:940` | S13.2b |
| `src/lib.rs:941` | S13.2b |
| `src/lib.rs:942` | S13.2b |
| `src/lib.rs:979` | S13.2c |
| `src/lib.rs:980` | S13.2c |
| `src/lib.rs:981` | S13.2c |
| `src/lib.rs:982` | S13.2c |
| `src/lib.rs:983` | S13.2c |
| `src/lib.rs:984` | S13.2c |
| `src/lib.rs:988` | S13.2c |
| `src/lib.rs:989` | S13.2c |
| `src/lib.rs:990` | S13.2c |
| `src/lib.rs:992` | S13.2c |
| `src/lib.rs:997` | S13.2c |
| `src/lib.rs:1002` | S13.1 |
| `src/lib.rs:1003` | S13.2d |
| `src/lib.rs:1004` | S13.2d |
| `src/lib.rs:1005` | S13.1 |
| `src/lib.rs:1007` | S13.2d |
| `src/lib.rs:1009` | S13.2d |
| `src/lib.rs:1010` | S13.2d |
| `src/lib.rs:1011` | S13.2d |
| `src/lib.rs:1012` | S13.2d |
| `src/lib.rs:1014` | S13.2d |
| `src/lib.rs:1015` | S13.2d |
| `src/lib.rs:1016` | S13.2d |
| `src/lib.rs:1017` | S13.2d |
| `src/lib.rs:1018` | S13.2d |
| `src/lib.rs:1021` | S13.2d |
| `src/lib.rs:1022` | S13.2d |
| `src/lib.rs:1040` | S13.1 |
| `src/lib.rs:1042` | S13.1 |
| `src/lib.rs:1051` | S13.2h |
| `src/lib.rs:1052` | S13.1 |
| `src/lib.rs:1053` | S13.2h |
| `src/lib.rs:1054` | S13.2i |
| `src/lib.rs:1055` | S13.2i |
| `src/lib.rs:1057` | S13.2j |
| `src/lib.rs:1058` | S13.1 |
| `src/lib.rs:1078` | S13.1 |
| `src/lib.rs:1079` | S13.1 |
| `src/lib.rs:1080` | S13.1 |
| `src/lib.rs:1268` | S13.2e |
| `src/lib.rs:1293` | S13.2e |
| `src/lib.rs:1294` | S13.2e |
| `src/lib.rs:1297` | S13.2e |
| `src/lib.rs:1316` | S13.2f |
| `src/lib.rs:1317` | S13.2f |
| `src/lib.rs:1318` | S13.2f |
| `src/lib.rs:1319` | S13.2f |
| `src/lib.rs:1320` | S13.2f |
| `src/lib.rs:1770` | S13.2g |
| `src/main.rs:9` | S13.3a |
| `src/main.rs:51` | S13.3b |
| `src/menu.rs:41` | S13.3c |
| `src/menu.rs:48` | S13.3c |
| `src/runtime.rs:248` | S13.3f |
| `src/runtime.rs:249` | S13.3f |
| `src/runtime.rs:251` | S13.3f |
| `src/runtime.rs:252` | S13.3f |
<!-- 109 rows -->
