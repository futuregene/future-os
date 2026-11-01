# host (Rust, `cargo test -p future-host --lib`) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The llvm-cov
summary's `Lines` column is not the acceptance number (it adds ghost lines).

**Lane** — `future-host`'s **lib** unit tests only. The integration-test binaries and the
`#[ignore]`d real-network legs are measured by their own lanes; this file is about the lib.

**Last measurement**: `test/cov-host` at `deb03ec4a`, rustc **1.99.1** (`rust-toolchain.toml`),
`HOME=target/test-home`, `ulimit -n 8192`. Artifacts (gitignored) are in
`coverage/2026-11/after/host/`: `round7-A-lcov.info` (pass A, the numbers quoted below),
`round7-B-lcov.info` (an independent repeat, §1.1), `round7-run.log` / `round7-B-run.log`, the
per-line inventories `round7-A-missed.json` / `round7-B-missed.json`, the bucket arithmetic in
`buckets7.py`, the §4 rows in `open-ranges7.py`, the closing-edge list in
`closing-edges-round7.txt` and the run-to-run line diff in `flaky-lines.py`. Superseded by this
round: `round6-A/B-*` (2505), `round5-A/B2-*` (2553), `base3-*` (2589) and `final-*` (2589).

The lane record beside this file — `host-lane.json` — carries the same numbers, and
`scripts/coverage/lane-check.py` fails when the record, this doc and the lcov artifact disagree
(§8). Run it after every re-measurement; it is the cheap gate this lane can actually carry.

```bash
export PATH=/Users/geilige/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH
unset CARGO_BUILD_TARGET CARGO_TARGET_DIR CARGO_INCREMENTAL
export HOME=$PWD/target/test-home; ulimit -n 8192

cargo llvm-cov clean --manifest-path Cargo.toml   # NOT --workspace
rm -rf target/llvm-cov-target
cargo llvm-cov -p future-host --lib --no-report
cargo llvm-cov report --lcov --output-path /tmp/host-lcov.info

# Metric A with nothing but the lcov file (works from a clean clone):
awk '/^SF:/{f=substr($0,4)} /^DA:[0-9]+,0$/{split(substr($0,4),a,","); print f":"a[1]}' \
  /tmp/host-lcov.info | grep '/host/src/' | wc -l
```

The run is also the gate: **1955 passed, 0 failed, 11 ignored** in 435.26 s, then `report rc=0`
(`round7-run.log`).

## 1. Before / after, and where every remaining line sits

Both columns are `DA:0` over `host/src/`, same lane, same recipe. "before" is `round6-A-lcov.info` —
the previous round's measured tree (`31f46778d`), which is what this round's work started from.

| | before (`round6-A-lcov.info`) | after (`round7-A-lcov.info`) | Δ |
|---|---:|---:|---:|
| **lane total** (`host/src/**`) | 2505 | **2464** | **−41** |
| switch-lane files (§2.1) | 1884 | 1884 | 0 |
| **inside in-scope files, switch lane only** (§2.2, §2.3) | 132 | 132 | 0 |
| **in-scope** (lane total − §2.1 files − §2.2/§2.3 lines) | 489 | **448** | **−41** |

Every one of the 2464 remaining lines is in exactly one bucket (`buckets7.py` prints this; the
four numbers sum to the total):

| bucket | lines | what it is |
|---|---:|---|
| **switch lane: their own files** | **1884** | §2.1 — five files only the network legs execute |
| **switch lane: inside in-scope files** | **132** | §2.2 (125: `tasks.rs`'s two `#[ignore]`d tests, 83; `remote/test_support.rs`'s `reconnect_secure_channel`, 42) plus §2.3 (7: the `fixture_mode()` arms that `cfg!(test)` makes constant) |
| **waived** | **90** | §3 rows W1-W7 (21), W8's closing edges (20), W9-W12 (33) and W13-W17 (16) |
| **open** | **358** | §4 — real gaps. **Not waived.** |

So the campaign figure for this lane today is **358 open**, not 0: this pass closed 41 in-scope lines
by measurement and re-judged 15 more into §3 (below), and 2016 of the residual are another lane's to
measure. The two are different moves and the arithmetic keeps them apart: `414 − 358 = 56`, of which
**41 were executed** and **15 were already executed but mis-attributed to §4** (W14-W17).

`buckets7.py` asserts, before it counts, that every §3 waiver line and every §2.3 fixture arm is
still `DA:0` — a waiver this measurement would have falsified, or an arm it would have covered,
fails the run instead of being silently retired. All 90 waived lines and all 7 arms held.

Where this pass's 41 closed lines came from (per file, before → after, raw `DA:0` including the
waived lines of the same file):

| file | before | after | what closed it |
|---|---:|---:|---|
| `remote/control.rs` | 86 | **45** | `497236b94` — the control channel driven by the crate's own service fixtures: `spawn_host`'s loop started for real (directory created, abandoned request swept, live request answered off disk, aborted through the task registry), a Desktop that is *on* minting an invitation from a scripted platform, a removal through `unpair_slot`, a takeover answered and a takeover refused, the terminal switch stored both ways (§4.1). Its open share goes **85 → 29**, all of it the client role's changing half (§4.1), and **15** lines of it move to §3 |

No other file moved this round (the per-file diff printed exactly one row).

Earlier passes, for the record: `shadow_review/snapshot.rs` 24 → 1,
`store/migrations.rs` 17 → 3, `agent_bridge/observer.rs` 39 → 29, `remote/control.rs` 88 → 86
(`de46de229`, `9653062e3`, `2e8130686`, `31f46778d`); `remote_host/business/mod.rs` 11 → 0,
`store/test_support.rs` 5 → 0, `tasks/service.rs` 5 → 1, `events.rs` 5 → 2, `remote_host/mod.rs` 3 →
0, `remote_peer/host.rs` 5 → 2, `runtime.rs` 7 → 4, `startup.rs` 7 → 4, `store/threads.rs` 3 → 1,
`scheduler.rs` 2 → 3, `shutdown/services.rs` 17 → 16 (`a8d871797`, `5859070e1`, `b4b8f605e`,
`929a4a798`, `ae228f549`); `agent_bridge/queries.rs` 12 → 0, `remote/mod.rs` 14 → 0,
`agent_host.rs` 58 → 28, `agent_bridge/run_control.rs` 14 → 2 and `agent_bridge/stream.rs` 14 → 4
(`188e08161`); `agent_bridge/reconciliation.rs` 39 → 2, `application/threads.rs` 15 → 3 and
`agent_client.rs` 16 → 7 (`f5b44775a`); `store/reconcile.rs` 13 → 4 and
`agent_bridge/observer.rs` 40 → 39 (`5b88f9f9d`); `agent_bridge/queries.rs` 69 → 12,
`remote/control.rs` 138 → 88, `remote_host/pairing.rs` 120 → 22 and `remote/relay.rs` 124 → 4
(`75946a7ed`); `remote_platform.rs` 18 → 10 and `agent_bridge/config.rs` 24 → 4 (`ed6547572`);
`application/threads.rs` 40 → 22, `application/runs.rs` 14 → 0, `application/skills.rs` 26 → 3 and
`test_support/timing.rs` 40 → 1 (`0b9c945c3`).

### 1.1 Same-number reproduction: the measurement, run twice

Pass B (`round7-B.sh`) runs the identical recipe on the same instrumented build with its
`.profraw` files deleted first, into an independent lcov file. Both passes are green:
**1955 passed / 0 failed / 11 ignored** in 435.26 s (A) and 426.38 s (B).

| | pass A (`round7-A-lcov.info`) | pass B (`round7-B-lcov.info`) |
|---|---:|---:|
| lane total `DA:0` | **2464** | **2464** |
| switch-lane files | 1884 | 1884 |
| in-scope | 448 | 448 |
| lines that differ between the runs | — | **2**, both named in §1.2 |

Two lines moved and the total did not, because they swapped (§1.2). The pair before this one
agreed line for line; the honest claim is unchanged: **the measurement is stable to a couple of
named lines, and every line that moves is named.**

Three passes in this round were **discarded rather than read**, all kept on disk under
`-DISCARDED` names so the discard is auditable:

- `round7-A-DISCARDED-parked-*` is the pass that **parked at 0 % CPU** after twelve tests (§5.17).
  Its `sample`, classified this round rather than only kept, is
  `round7-park-sample-2026-11-01.txt`. A parked pass is the same trap as a killed one: the tests
  that never ran leave their lines `DA:0` and the report looks complete — and this one's lcov was
  *written* (the harness reports the SIGTERM as a failure, and `report` still runs on the partial
  `.profraw`s), which is exactly how a killed pass comes to look green.
- `round7-A-DISCARDED-red-*` is a **red** pass — `1955 passed / 1 failed` — from the
  `remote_peer::testing` fixture that was dropped afterwards (§4.1). A green-looking lcov that had
  one failing test in it is not a measurement.
- `round7-control-DISCARDED-premeasurement-lcov.info` is the filtered run used to check one file's
  lines while the tests were being written (§5.15). It is fine for that and is never quoted as a
  lane number.

Earlier rounds' discards — `round6-B-DISCARDED-*`, `round5-B-DISCARDED-sigkill`,
`round4-A-DISCARDED-two-failures` — are the same rule applied before.

### 1.2 Lines that move between passes, named

| line | what it is |
|---|---|
| `agent_host.rs:502` | `server.as_mut().await?` in the shutdown-report arm of the start loop. Covered in pass B, `DA:0` in A — and in every round where it has moved. A race between the worker's shutdown report and the teardown it then awaits; named, not waived, and §4 keeps it |
| `shutdown/services.rs:20` | `return Err(error.into())` when `super::failure()` already holds a recorded failure. `DA:0` in pass B, covered in A: it depends on whether an earlier test in the same binary left a failure recorded, which is exactly the kind of line a whole-suite run cannot pin |

The honest claim stays: **the measurement is stable to a couple of named lines, and every line that
moves is named.**

### 1.3 The five refusal arms of the projection, and why the assertion is the cursor

`2e8130686` drives `handle_event_from` five ways. Each test asserts the return value **and** that
neither the run cursor nor the session cursor moved, because a refusal that advanced a cursor would
be silently skipped for the rest of the session — the event would be "handled", and never replayed.
Falsified one at a time: turning each of `1041`, `1048`, `1108`, `1115`, `1234`'s `return false`
into `return true` turns exactly one of the five red (`git show 2e8130686`).

## 2. Switch-lane lines (not this lane's business)

### 2.1 Their own files — 1884

The criterion is per file and it is the same one for all five: **every test in the file carries an
`#[ignore]` naming the process, broker or snapshot it needs**, so a default `cargo test` never
executes a line of it — the lines are not unreachable, they are unrun by *this* lane.

| file | lines | the `#[ignore]` reason in the source |
|---|---:|---|
| `remote/real_network.rs` | 788 | `real network: needs nats-server + a future-agent (docs/testing/real-network.md)` — a two-process exercise |
| `remote/download_tiers_network.rs` | 449 | `real network: needs nats-server (docs/testing/remote-download-tiers-live-2026-10-26.md)` |
| `remote/verify_e2e.rs` | 434 | `real broker: needs VERIFY_E2E_NATS_URL …` / `measurement: needs VERIFY_E2E_JOURNAL/…` |
| `remote/relay_e2e.rs` | 173 | `needs a running future relay serve (docs/guide/self-hosted-relay.md)` |
| `remote_host/sync_measurement.rs` | 40 | `requires isolated DB snapshot; run scripts/measure/measure-sync-browser.py` |

Byte-identical before and after (`1884 → 1884`): the campaign's rule for this lane is not to write
tests or change measurement for them.

### 2.2 Inside in-scope files — 125

- `host/src/tasks.rs:1918-2025` — **83 lines**: the two `#[ignore]`d tests
  (`a_queued_run_against_a_real_agent_ends_with_a_usable_answer`, and the manual
  `serve_pending_runs_until_idle` harness), which need a real Agent, a real model and credit. Their
  `#[ignore]` reasons are in the source.
- `host/src/remote/test_support.rs:294-339` — **42 lines**: `reconnect_secure_channel`, an IK
  reconnect helper whose only caller is `host/src/remote/verify_e2e.rs:2687`, i.e. the e2e leg.

### 2.3 The `fixture_mode()` arms a lib build cannot take — 7

`remote_peer/host.rs:40`, `remote/web_server.rs:16,39`, `remote/diagnostics.rs:26,28`,
`remote/supervisor/state.rs:37,48`.

Each is the arm taken when `crate::remote::fixture_mode()` is **false**. That function is
`cfg!(test) || FIXTURE_MODE.load(…)` (`host/src/remote/mod.rs:72-80`), and this lane compiles the
crate under `cfg(test)`, so the answer is a constant `true` here and the other arm is unreachable
by construction — including `web_server.rs:39`, where the `||` short-circuits before
`web_client_enabled_for_platform(…)`, and `diagnostics.rs:26,28`, the close and fallthrough of a
block that always returns.

**Falsifier**: `cargo llvm-cov -p future-host --tests`. There the lib is compiled *without*
`cfg(test)` (the integration binaries link it), `FIXTURE_MODE` starts `false`, and every one of
these seven lines executes; the fixture-mode arm it replaces then reads 0 instead. This is why the
lines are reported as another lane's rather than waived: a lane exists that measures them.

## 3. Waived constructs — 75 lines

Each row names the mechanism, the test that pins the neighbouring covered lines (the positive
control), and a falsifier. A waiver that cannot name its falsifier is not evidence
(`docs/testing/waiver-ledger.md`). `W1`–`W8` are the classes defined in
`.future/memory/coverage-llvm-cov.md`: W1 cfg/​platform · W2 defensive dead arm · W3 OS failure
injection · W4 race window · W5 attribution artifact · W6 ghost line · W7 mock closure ·
W8 dead producer/deliberately-unexecuted line.

| # | file:line | lines | class | construct | positive control | falsifier |
|---|---|---:|---|---|---|---|
| W1 | `host/src/remote/relay.rs:469,587,594,597` | 4 | W2 | the test fixture's own failure arms: `Ok(_) => panic!("{what}")` in `a_refused_grant`, the mock relay's socket-read `expect`, and the `continue` for a buffer that never held a request head | `remote::relay::tests::register_signs_the_room_request_and_mints_a_grant` and `…::a_relay_that_is_not_listening_is_a_transport_failure` run the same fixture on its success and error paths | hand the helper an `Ok(_)`, or open a connection that writes nothing: 469 and 597 are taken |
| W2 | `host/src/remote/control.rs:2135` | 1 | W2 | `Ok(_) => panic!("{what}")` in `a_refused_peer_request` (the line moved +1 when the control tests module took a `use` line) | `remote::control::tests::perform_peer_reads_the_list_and_asks_which_machine` uses the same helper on its `Err` arm | call it with an `Ok(PeerOutcome)` — the suite fails there |
| W3 | `host/src/remote/trust.rs:96-98` | 3 | W6 | `#[cfg(not(test))] fn bundle_path()`: the body has no executing caller in a `--lib` build; its `DA:0` record comes from the crate's second (dependency) compilation, the "lib 双编译合并" shape in the memory note | `bundle_at`'s own tests cover the same snapshot-and-fallback logic | an integration-test binary calling it — another lane |
| W4 | `host/src/remote/test_support.rs:784,789` | 2 | W1/W6 | the `else` arms of `cfg!(target_os = "macos")` in the mock's `probe_sandbox` answer (`"unsupported_platform"`, `"none"`) — this machine is macOS, so the other platform's text never executes | `agent_bridge::queries::probe_tests::the_sandbox_probes_ask_the_agent_and_map_its_answer` covers the same answer's mapping | run the suite on Linux: those two lines execute and the macOS ones read 0 |
| W5 | `host/src/remote/test_support.rs:801-808` | 8 | W8 | the mock's default `probe_thinking_level` answer — evaluated only when a test leaves the command unscripted | `agent_bridge::config::tests::a_thinking_level_probe_forwards_the_request_and_keeps_a_refusal_as_an_answer` owns that command via a script, so the default is bypassed on purpose | drop one `probe_thinking_level` script from any test: the default arm runs |
| W6 | `host/src/remote/test_support.rs:356-358` | 3 | W8 | `RecordingSink::is_empty` / `len` — accessors no handler test currently calls | the same type's `push`/`drain` are used by the handler tests | one assertion in any handler test covers all three |
| W7 | `host/src/remote/test_support.rs:1963,2193,2710` | 3 | W2 | failure arms inside tests: a `violations.push(…)` for a doc-contract breach, its `join` in the assert message, and `Err(error) => panic!(…)` in a deferred-arm matcher | the same tests pass, so their happy paths execute | break the doc contract the test checks, or answer the deferred call with a failure |
| W8 | 20 lines, listed in `coverage/2026-11/after/host/closing-edges-round7.txt` (`store/legacy_history.rs:24,81,212,260,265`, `store/migrations.rs:39,76,139`, `agent_bridge/stream.rs:318,388`, `agent_bridge/import.rs:632`, `agent_bridge/presence.rs:112`, `application/threads.rs:420`, `store/cleanup.rs:456`, `store/deletions.rs:26`, `store/managed_cleanup.rs:28,38`, `store/review_snapshots.rs:203`, `store/runs.rs:184`, `store/approval_assessments.rs:63`) | 20 | W6 | a line that holds **nothing but** a call's closing error edge (`)?;`, `})?;`, `)?`) — "多行调用把 `)?;` 单独落一行 → 该行只有错误臂 → `DA:0`" (`.future/memory/coverage-llvm-cov.md`) | the statement itself ran: the line directly above each of these 20 reads `DA>0` (checked mechanically by `buckets7.py`). The closing-edge lines whose neighbour is *also* uncovered stay in §4 (`agent_bridge/config.rs:202`, `agent_client.rs:178,209`, `remote/web_server.rs:61`, `remote_host/pairing.rs:293`, `remote_platform.rs:53`, `remote/test_support.rs:67`, `agent_bridge/observer.rs:1049`) | put one of these calls on a single line (shrink its argument list into rustfmt's 60-column `fn_call_width`) and the line disappears from the next report with no test change; conversely a caller that actually forces the error edge — a closed store, an unreadable file — covers it |
| W9 | `host/src/remote_host/business/catalog.rs:104-106,122-130` | 11 | W2 | the two "the fork produced nothing usable" arms of the phone's `fork_session`, plus the `get_thread` failure beside them. All three sit behind one invariant: `agent_bridge::fork_agent_session` refuses a fork whose session id came back empty (`agent_bridge/session.rs:316-318`) and *creates* the thread itself, with that session id bound, before returning its id (`session.rs:373-384`, `get_or_create_thread_for_agent_session(… agent_session_id: Some(new_session_id))`) | `business::catalog::dispatch_tests::fork_session_creates_and_reports_the_child_conversation` and `…::a_refused_fork_is_reported_with_the_agents_reason` drive the same call on the success path and the refusal path, and both read the child back through the same `get_thread` | make `fork_agent_session` return an id it did not persist (a session the Agent forked on another machine, or a fork that returns the parent's id) — then 122-128 answers and all eleven lines execute |
| W10 | `host/src/remote_host/business/settings.rs:148-150,177-181` | 8 | W1 | the sandbox probe's non-answer arms: `product_sandbox_status()`'s `Err` (148-150, 179-181) and `product_sandbox_available()`'s `Ok(false)` (177-178). On macOS that function is `#[cfg(target_os = "macos")] { Ok((true, None)) }` (`settings.rs:301-310`): the sandbox is a constant there, so neither the `Err` nor a `false` availability has an input in this lane | `business::settings` tests drive `get_settings` and the settings write through `product_sandbox_available()`, so the macOS arm itself is covered | run the lane on Linux or Windows, where the answer comes from `agent_bridge::probe_sandbox()`: a probe failure reaches 148-150 and 179-181, and a host without a sandbox reaches 177-178 |
| W11 | `host/src/shutdown.rs:186-187,204-208` | 7 | W3 | the two `std::thread::Builder::new().spawn(…)` failures of the stop path: the cleanup watchdog (186-187) and the cleanup worker (204-208). Thread creation fails only when the OS refuses it | `shutdown`'s tests run the same `stop()` to completion, including the worker's finish/progress reporting that the arms surround | exhaust the process's thread budget (lower `RLIMIT_NPROC`/`ulimit -u` to what is already in use) and call `stop()`: `pthread_create` fails, `spawn` returns `Err`, and both arms answer |
| W12 | `host/src/remote_host/business/providers.rs:58-64` | 7 | W2 | `serde_json::to_value(view)` in `reply_view`, where `view: agent_providers::ProvidersView` is `Vec<BuiltinProvider>` + `Vec<CustomProvider>` of `String`/`bool`/`usize` fields (`agent_providers/mod.rs:59-76`). `to_value` fails only on a non-string map key or a non-finite float, and neither type occurs in the view | every provider test answers through `reply_view`, so the same `to_value` succeeds on the same types | add a field whose `Serialize` can fail (an `f64` that can be NaN, or a map keyed by a struct) — the arm then has an input and answers |
| W13 | `host/src/shadow_review/snapshot.rs:266` | 1 | W6 | the `paths = paths.len(),` field value of `try_stage`'s "skipped paths that no longer exist" warning. `tracing`'s field expressions are evaluated only inside the callsite's enabled block, and this one has **no hit counter of its own**: the region belongs to the callsite construction on `264` | `shadow_review::snapshot::tests::stage_warns_with_the_candidate_count_when_a_pathspec_matches_nothing` installs a thread-scoped subscriber and **asserts the recorded value** (`paths = 3`) — so the expression runs, on a line the report calls 0. The same walk through `enabled`/`new_span`/`record`/`record_follows_from`/`enter`/`exit` leaves the fixture itself with no unexercised method | `cargo llvm-cov -p future-host --lib` with that test present still prints `266` as `DA:0` while the assertion passes (measured again this round, `round7-A-missed.json`); delete the assertion and the run fails at `Some(3) != None` — the line is *executed*, so this is attribution, not reachability |
| W14 | `host/src/remote/control.rs:584-586` | 3 | W3 | `directory()`'s failure arm: `Err(error) => { eprintln!(…); return }`. It answers only when the FutureOS data root cannot be resolved at all — no `HOME`, no `USERPROFILE`, no passwd home — which no fixture can arrange without breaking every other test in the same binary | `…::the_control_loop_creates_its_directory_and_answers_what_it_finds` and `…::the_control_loop_gives_up_on_a_directory_it_cannot_create` run the same `directory()` call on its `Ok` path, and the second drives a **refused** `create_dir_all` into its own report-and-return | start the loop in a process whose home cannot be resolved (`env -u HOME -u USERPROFILE` with no passwd entry): `future_dir()` fails and 584-586 answer |
| W15 | `host/src/remote/control.rs:839` | 1 | W2 | the takeover match's catch-all `Err(error) => Err(error)`. `Supervisor::takeover` has exactly two outcomes — `device_offline` when this process holds no runtime for the device, and the status it just took | `…::a_takeover_hands_the_machine_over_or_names_a_device_with_no_bridge` drives both of those, and asserts the `device_offline` arm's own sentence | add a third failure to `Supervisor::takeover` (a status that cannot be projected, a `publish_control` that fails): the catch-all then has an input |
| W16 | `host/src/remote/control.rs:989-993` | 5 | W2 | the client role's unknown-op refusal, behind the router's own decision: `serve_one` sends a request to `perform_peer` only when `is_peer_op` accepts it, and `perform_peer` answers all five operations that function accepts | `remote::control::tests::the_client_role_operations_are_told_apart_from_the_host_ones` pins `is_peer_op`'s membership — the two sets agreeing is what makes this arm unreachable | add an op to `is_peer_op` without an arm for it (say `OP_ENABLE`): the router forwards it and 989-993 answer instead of the request reaching `perform` |
| W17 | `host/src/remote/control.rs:1238-1243` | 6 | W8 | the operation future inside `an_unknown_operation_is_refused_without_performing_anything`: the test hands `answer` a future that would set `performed = true` and return a status, and its whole point is that the future is **never polled** | the neighbouring `answer` tests hand the same shape to the same call and it does execute — `a_status_without_an_invitation_is_an_error_reply` runs `async { Ok(refused) }` to its own assertion | poll it: give that test a *known* op (`OP_STATUS`) and all six lines execute — which is exactly what its `assert!(!performed)` exists to prevent |

Waived total: **90** (21 in rows W1-W7, 20 in W8, 33 in W9-W12, 1 in W13, 15 in W14-W17). Nothing else in §4 is
claimed as waived. W14-W17 are this round's: the lines that were counted as *open* last round and
are executed-but-unreachable from this lane for the reasons in their rows.

Round 8 proposes one more, and does not move the number: `agent_bridge/observer.rs:1049` is the
closing brace of the hydration refusal's `if let Err` block, whose only statement is `return false;`
— no input can execute it, and its neighbours (1047-1048) are covered, which is W8's own test. It
moves into §3 as W18 on the next full pass, when `waived` 90 → 91 and `open` 358 → 357 can be
re-measured together (§9.1).

## 4. Open residual — 358 lines in 63 files

**Not waived.** Every row is the measurement plus whatever has actually been read; "not yet read"
means exactly that. Rows are `DA:0` line numbers in the tree this file records (`deb03ec4a`).

| file | open lines | `DA:0` | mechanism |
|---|---:|---|---|
| `remote/control.rs` | 29 | 939-941,945-946,951-952,960-966,970-975,981-987,997-998 | the client role's **changing half** (`perform_peer`'s connect / disconnect / takeover / remove). **Closed in round 8** (§9.2): the fixture's blocker was a `suspended` host role that outlived its own fixture, not the pairing; the focused run measures all 29 as covered |
| `agent_host.rs` | 29 | 144,173-174,177,216-217,311-313,334,363,435,443,451,487,502-503,531,539,665,762-764,766-767,815,890,1027-1028 | `is_stopping()` arms (144, 177 — closing the process-wide admission is one-way for the test binary), the recovery closure's unfinished-retirement arm (173-174), the handle-level idle admission (216-217, needs a live `AgentHandle`), worker-spawn failure (311-313), `Stopped` with no yield reason (334), the external-mode validation (363), attach-only refusals (435, 443, 451), the stop/serve precedence arms (487, 503, 531, 539, 665), the fixture harness's own failure arms (762-767, 815, 890, 1027-1028), and the race-attributed `502` (§1.2) |
| `agent_bridge/observer.rs` | 29 | 369-376,398-403,790-796,806-812,1049 | the discovery pass's `Ok(n)`/`Err` arms for the three reconciles (369-376, 398-403), the source-verification refusal and its backoff (790-796), the re-verified-source reset (806-812), and `1049`, the brace after the hydration refusal's `return`. **28 of the 29 are closed in round 8** (§9.3) by driving one real discovery tick and two real attach cycles; `1049` is the one left, and it is attribution, not a statement (W18, §3) |
| `remote_host/pairing.rs` | 22 | 292-293,372,490-492,503-505,540-541,770-776,831-833,1682 | leftover refusal arms around the platform calls |
| `cli_tool.rs` | 19 | 84,93,100,105,255-257,326-328,333,345,385-387,402,705,732,832 | the `cfg(windows)` executable name (84, W1-shaped but left open: the falsifier is a Windows lane, not a waiver), the `"bundled"`/`"development"` source labels (93, 100 — the second is produced only by the `cfg(not(test))` candidate that this lane compiles out), the 5 s failed-probe cache renewal (105), the deadline-free constructor no test calls (255-257), the probe's own error and post-exit limit arms (326-328, 333, 345), the discovery-failure path (385-387), a closing brace (402), and two fixture asserts' message arguments (705, 732, 832) |
| `remote/trust.rs` | 17 | 64,66,68,70-71,73,75,87,91-93,169-174 | `bundle_with_extra`'s arms behind a `OnceLock` (64-93 — one closure execution per process, so at most one arm per run), and `load_platform_certs`'s error collection (169-174) |
| `shutdown/services.rs` | 16 | 47-49,68,70-71,73,75-76,81,93,96,98-99,280,321 | service shutdown error arms and two closing braces |
| `tasks.rs` | 15 | 126-128,228-230,234,328,439,476,482,572-573,1859,2176 | store-open / delete / thread-load failure logs |
| `remote_platform.rs` | 10 | 49-53,225,233,246,326,392 | the `from_pem_bundle` error arm (measured unreachable by `remote_platform::tests::a_relay_ca_that_cannot_be_used_is_named`) plus fixture arms |
| `remote_host/read_pages.rs` | 9 | 70,377-384 | one `break` in the page loop plus a fixture sink that must never be used |
| `agent_bridge/headless.rs` | 9 | 158-164,297,310 | the `JoinError` arm that turns a panicked prompt task into a user-facing reason, and two harness arms |
| `shadow_review/repository.rs` | 8 | 392,415,441,456,460,465,477,483 | not yet read |
| `remote_host/business/prompt.rs` | 7 | 148-154 | the `accepted_rx.await` `Err` arm — the acknowledgement channel closed before the prompt was accepted; no fixture drops that sender yet |
| `agent_client.rs` | 7 | 177-178,192,208-209,397,556 | the two `JoinError` arms (a panicking verify/connect task), plus the deliberate `panic!` inside a test's losing-installer closure and one `matches!` line |
| `remote/pairing.rs` | 6 | 53,55,62,75-77 | conversion arms and the `load_creds_checked` delegation |
| `agent_bridge/session_events.rs` | 6 | 195-197,212-214 | deletion-source verification and session-removal failure logs |
| `store/legacy_history.rs` | 5 | 131,145,164,218,229 | legacy recovery arms |
| `skills.rs` | 5 | 75-79 | the skill-guide fetch's success path (needs an HTTP fixture for the platform call) |
| `remote/commands.rs` | 5 | 1503,1508-1509,1583,1602 | not yet read |
| `crash_log.rs` | 5 | 161,464,472-474 | the child-process fixture's broken-stderr variant (`161` is the `path.parent()` `None` arm) |
| `test_support/agent.rs` | 4 | 145,492,766,915 | not yet read |
| `test_support.rs` | 4 | 67,83-85 | `configure_entry_fixture()` (83-85) is the switch the entry lane turns on; calling it from a lib test would assert a `cfg!(test)` constant that is already true, so it is left open rather than covered by an assertion that cannot fail. `67` is an assert's message argument |
| `store/reconcile.rs` | 4 | 60-62,149 | the `record_thread_source` failure log, and the legacy-history skip |
| `store/cleanup.rs` | 4 | 440,444,466-467 | not yet read |
| `startup.rs` | 4 | 216,365-367 | the watchdog thread-spawn failure (216) and a test's reporter closure that the failing launch deliberately never calls (365-367) |
| `runtime.rs` | 4 | 290-291,293-294 | a test's task bodies that the frozen registry deliberately never polls |
| `future_login.rs` | 4 | 345-346,398,475 | not yet read |
| `agent_bridge/import.rs` | 4 | 190,458,535,595 | not yet read |
| `agent_bridge/config.rs` | 4 | 199-202 | not yet read (a closing edge with an uncovered neighbour) |
| `scheduler.rs` | 3 | 285,290,438 | test assert message arguments and a `panic!` for a schedule that must never poison twice |
| `remote_host/files.rs` | 3 | 386,1241,1279 | not yet read |
| `remote_host/business/tasks.rs` | 3 | 194,197,1194 | not yet read |
| `remote/supervisor/start.rs` | 3 | 33,38-39 | the fixture-mode branch of `start_once` |
| `remote/secure.rs` | 3 | 39-41 | not yet read |
| `instance.rs` | 3 | 24,53,80 | the no-override identifier (24), and two `directory.parent()` guards that need a path with no parent |
| `files.rs` | 3 | 637-639 | not yet read |
| `application/threads.rs` | 3 | 117,147,421 | `update_thread_*`'s agent-propagation block closers (117, 147: the brace after the propagation call, reached only when the call is skipped) and one brace behind a `?` |
| `application/skills.rs` | 3 | 48-50 | the `get_skill_guide` delegation (needs the platform HTTP fixture) |
| `store/workspaces.rs` | 2 | 60,914 | not yet read |
| `shadow_review/maintenance.rs` | 2 | 69,100 | the `}` after a failed consistency check and the same for interrupted-run recovery |
| `remote_peer/transfer.rs` | 2 | 45-46 | not yet read |
| `remote_host/command_diagnostics.rs` | 2 | 71,136 | a binary reply's byte count and the `replied=none` branch |
| `remote_host/business/history.rs` | 2 | 305,325 | the `get_event_content_range` dispatch arm and its failure paths |
| `remote/web_server.rs` | 2 | 60-61 | the `local_addr()` failure arm |
| `events.rs` | 2 | 203,226 | two `panic!` arms inside tests (a `let … else` that the passing test never takes) |
| `environment.rs` | 2 | 67,125 | the previous-log preservation failure report, and `File::open` failing for a reason other than `NotFound` |
| `agent_bridge/stream.rs` | 2 | 476,1539 | one `break` in the collector's re-attach loop (476) and a test's `panic!` for a fold that must end (1539) |
| `agent_bridge/run_control.rs` | 2 | 141-142 | two closing braces after a block that ends in `return` (141-142) — attribution, not a statement |
| `agent_bridge/reconciliation.rs` | 2 | 488-489 | the two block-closing braces after `settle_from_agent_terminal` `return`s |
| `agent_bridge/prompt.rs` | 2 | 235,258 | a conversation with preserved history but no writable session, and a recovery archive whose entries are all non-conversational |
| `workspaces.rs` | 1 | 64 | `delete_workspace`'s empty thread list — the `has_presentation()` false arm, which reads 0 only when no test has attached a sink yet (order-dependent) |
| `test_support/timing.rs` | 1 | 173 | not yet read |
| `tasks/service.rs` | 1 | 599 | an assert's message argument in the `without_an_override` test |
| `store/threads.rs` | 1 | 1899 | a test's `remove_dir_all` guard, reached only when the generated directory already exists |
| `store/approval_assessments.rs` | 1 | 172 | not yet read |
| `remote_peer/runtime.rs` | 1 | 355 | not yet read |
| `remote_peer/host.rs` | 1 | 158 | the assert-message argument of the `convert` test |
| `remote_host/session_files.rs` | 1 | 74 | a `metadata()` failure between listing and stat |
| `remote_host/business/transfers.rs` | 1 | 59 | not yet read |
| `error.rs` | 1 | 106 | a remote-client error → `host::Error` conversion arm |
| `build_info.rs` | 1 | 171 | not yet read |
| `auth_store.rs` | 1 | 269 | not yet read |
| `agent_bridge/presence.rs` | 1 | 86 | not yet read |

### 4.1 The `spawn_host` block: measured, not waived

Round 6 judged this block **OPEN** and named a supervisor fixture as what would measure it. Round 7
builds the fixture; the block is now measured except for three lines, and §4 holds no part of
`remote/control.rs`'s loop at all.

- **The loop itself.** `the_control_loop_creates_its_directory_and_answers_what_it_finds` starts the
  real `spawn_host()` with the data directory **not yet created**, one abandoned request already on
  disk beside a live `status` request, waits for the reply file a terminal would poll, reads it back
  with `read_reply`, and asserts the projection came back — then that the sweep removed the request
  nobody can be waiting for, that the served one was consumed, and (on unix) that the directory is
  `0700`. It aborts the loop through the task registry the shutdown paths use
  (`crate::runtime::cancel_test_tasks`), which is the only cancel path a process-lifetime loop has.
- **The failure arm.** `…_gives_up_on_a_directory_it_cannot_create` puts a *file* at `run/`, starts
  the loop, asserts the directory was never created, then frees the path and leaves a request: nothing
  answers it, which is what proves the loop returned instead of polling a directory it never made.
  `a_directory_that_cannot_be_read_lists_no_requests` covers `requests()`'s unreadable-directory arm.
- **The `perform` arms.** `an_invitation_from_a_desktop_that_is_on_reaches_the_platform` (767-777),
  `a_removal_reaches_the_running_supervisor_and_the_book` (792),
  `a_takeover_hands_the_machine_over_or_names_a_device_with_no_bridge` (831-837) and
  `the_terminal_switch_turns_the_desktop_off_and_on_and_stores_it` (847-856) drive the host role
  through a real supervisor over `FakeNats`, a scripted `MockPlatform` and a pairing book on disk —
  the services the arm *is* glue to, not a stub for `perform`.
- **What is left of the file — 16 lines, all §3 rows.** The `directory()` failure (W14: no
  `HOME`, no `USERPROFILE`, no passwd home), the takeover match's catch-all (W15), the client role's
  unknown-op refusal behind `is_peer_op` (W16), the operation a refused request proves is never
  polled (W17) and the fixture's own `Ok(_) => panic!` (W2).
- **Not measured, and why: the client role's changing half.** `perform_peer`'s connect / disconnect /
  takeover / remove arms were written against `remote_peer::testing::fixture` (a real host bridge on
  the in-process broker) and **measured red once in a full parallel run**: the fixture's `start_host`
  took its invitation while `remote::verify_e2e`'s driver body was stopping this process's
  supervisor, so the start was refused at the access epoch and answered `Stopped` — no code, panic at
  `remote_peer/testing.rs:68`. Green in isolation and in the whole `remote*` subset (647 tests), red
  at the end of the full suite. A one-in-N test is worse than fifteen uncovered lines, so the test was
  dropped rather than retried, and the hazard is named here instead: **the `verify_e2e` driver
  fixtures start and stop the process-global supervisor without taking `TEST_HOME_LOCK`**, which any
  remote-host test can lose against. That is the next thing to fix in this crate's test
  infrastructure — not a product seam, and not this lane's call.

Removed from the block this round (and now §3 rows rather than open): `767`, `776` (macro-attributed
condition and the brace after its `return`, both executed by the switch-off test above them),
`989-993`, `1238-1243`.

### What needs a product seam (judged OPEN, not waived)

Two blocks cannot be measured without changing product code to make themselves observable, or
without a parallel process. They stay open and the missing piece is named:

- `remote_host/business/prompt.rs:148-154` — the `accepted_rx.await` `Err` arm. Needs a way for a
  test to drop the acknowledgement sender; today it is an internal `oneshot` created inside the
  handler, so no fixture can reach it.
- `remote/trust.rs:64-93` — `bundle_with_extra`'s four arms inside one `OnceLock` closure, of which
  at most one runs per process. Needs the body extracted from the closure, which is product code
  written for the measurement (§5.8).

`remote/control.rs`'s supervisor block used to be the third: round 7's loop fixture measures it
(§4.1).

## 5. Rejected approaches (this lane)

1. **Classifying a file by "it has `mod tests`".** The earlier pass's method for 359 lines;
   measurement settled it (§6).
2. **Trusting an `after-lcov.info` whose run was killed or failed.** The first measurement of this
   campaign merged a corrupt profraw (`warning: … invalid file header` / `error: no profile can be
   merged`). Later instances of the same trap were thrown away instead of read:
   `round4-A-DISCARDED-two-failures.info` (a green-looking lcov from a run in which two new tests
   failed at their first assertion), `round5-B-DISCARDED-sigkill.info` (partial profraws from a
   `SIGKILL`), and this round's `round6-B-DISCARDED-superseded-*` (a pass over a tree the round
   then changed, §5.14).
3. **Adding `#[ignore]`.** Forbidden, and it would hide a gap behind the very mechanism that makes
   the switch-lane files read 0.
4. **Waiving `relay_trust_anchors`'s `from_pem_bundle` error arm by argument.** Tried as a
   measurement instead: three malformed inputs produced either one accepted certificate or an
   empty bundle. The arm has no reachable input from `--relay-ca`, and that is recorded by a test.
5. **Waiving `remote/test_support.rs:294-339` as a dead producer (W8).** Written down, then
   falsified before it shipped: `reconnect_secure_channel` is called by
   `host/src/remote/verify_e2e.rs:2687`. Reclassified into the switch lane (§2.2).
6. **Counting the switch-lane files as progress.** They are reported separately and deliberately
   unchanged (`1884 → 1884`).
7. **A "cover `spawn_host` by asserting its log lines" test.** The loop only logs on a
   data-directory failure, and any test that started it would leave a process-lifetime poll loop
   behind for every later test in the binary. Left open instead, now with the judgement in §4.1.
8. **Test-only seams for `remote/trust.rs`'s `bundle_with_extra`.** Extracting its body out of the
   `OnceLock` closure would make all four arms testable, but the extraction is product code written
   for the measurement. Left open rather than reshaping the function to be measured.
9. **Waiving `agent_host.rs:144,177` as "unreachable".** They are the `request_scope::is_stopping()`
   arms: falsifiable by closing process-wide command admission, which is one-way. Measured as open.
10. **A `--relay-ca`-style env fixture for `cli_tool::discover`'s `"development"` label.** The
    candidate that produces it is `#[cfg(all(debug_assertions, not(test)))]`, so this lane compiles
    it out. Recorded as open with that mechanism.
11. **Two passes in one shell call.** The suite needs ~7 minutes per pass and a sandbox call is
    capped at 600 s: a killed pass leaves partial `.profraw` files that look like a measurement.
    Both passes run detached, each with its own log. (Killing the script does **not** kill the
    `cargo-llvm-cov` child — `pkill -f cargo-llvm-cov` does, and that is what "discarded" has to
    mean: a pass whose children are still writing is not one to read.)
12. **Writing the capability-gate tests against the bridge mock and a `TestHome`.** One mock family
    per module: the bridge mock and the remote mock each re-point the process-wide
    `FUTURE_AGENT_GRPC_ADDR` at their own server, so a test in one family can have its command
    answered by the other's default script. Proven deterministic in the previous round; the rule
    stands.
13. **Asserting that a lib test binary has no presentation sink.** `events::attach` is a
    process-global `OnceLock` and `agent_bridge/observer.rs:2501` attaches one, so
    `has_presentation()` depends on which test ran first.
14. **A `tracing` subscriber with all seven `Subscriber` methods as a measurement fixture.** The
    first version of the W13 fixture left five of them (`new_span`, `record`, `record_follows_from`,
    `enter`, `exit`) unexercised — 22 newly uncovered lines, in this lane, bought by the test that
    was supposed to *reduce* the count. Measured and then reshaped instead of waived: the test now
    drives a span through every hook (`info_span!` + `record` + `follows_from` + `in_scope`), and
    the suite reads 0 for the fixture. What it cannot do is move `snapshot.rs:266` — the field
    evaluates (asserted) and the line still reads `DA:0`, which is the W13 row in §3.
15. **Reading a "complete-looking" lcov from a partial run.** This round's first look at the fixed
    tree was a filtered pre-measurement run (`cargo llvm-cov … -- shadow_review::snapshot`), which
    covers only that file's tests. It is fine for checking one file's lines and is never quoted as a
    lane number; the recorded pair is measured by the scripts, from a clean instrumented build. Its
    lcov is kept on disk as `round6-A-DISCARDED-premeasurement-lcov.info` for exactly that reason —
    it is the same number, and it is not the measurement.

16. **Reading a parked pass as a measurement.** This round's first B attempt never finished: after
    twelve tests it sat at 0 % CPU for seven minutes. `sample` (kept as
    `host-lock-parks-2026-11-01.txt`) names the parks and nothing more — every test thread waits on
    one of the two process-wide fixtures (`test_support/agent.rs:428` = `MOCK_LOCK`, 31 threads;
    `agent.rs:695` / `auth_store.rs:194` = `TEST_HOME_LOCK`, most of the rest) with the mock's
    runtime idle, which is the same signature as the previous round's unproven cycle. It is **not**
    this round's code: the parked tests are the `approval`, `config`, `config_observer`, `headless`
    and `import` families, none of which this round touched, and the same suite passed three times
    today (two measurements and a pre-measurement run). So this file claims no mechanism, records
    the frequency (one parked pass in three full runs) and applies the killed-pass rule: discard it,
    re-run, and keep the sample as evidence. Naming that cycle needs a lock-order probe the fixtures
    do not have; it is the one thing an operator should look at first if the suite ever stalls in
    CI.

17. **Calling the park "just the lock serializing the suite", and speeding the fixtures up.** Round 7
    parked again — one pass of two — on the same five families, and this time the dump was classified
    rather than only kept (`round7-park-sample-2026-11-01.txt`, `park-locks.py`): 100 threads, 0 % CPU
    for the whole 5 s window, **30 threads waiting to *take* a fixture and none of them holding one**
    (12 in `mock_agent_with_cleanup` = `MOCK_LOCK`, 18 in `TestHome::with_cleanup`/`HomeGuard` =
    `TEST_HOME_LOCK`), 63 in a tokio `park_condvar`, 4 in `kevent`, and the harness's main thread
    waiting for a completed test. The two readings that follow are different and must not be
    conflated:

    - **The suite's ~430 s is serialization, by design** (§8): a fixture holds `TEST_HOME_LOCK` for
      its whole lifetime because a test publishes `HOME` as a real environment variable. That is
      slow, not broken, and there is nothing to optimize in `approval`/`config`/`headless`/`import`.
    - **A park is not that.** Every thread that could make progress is queued behind a fixture whose
      holder is not on a CPU, so the holder is either a task **parked inside a tokio runtime while
      holding the guard** — the shape these tests deliberately have, `#[allow(clippy::await_holding_lock)]`
      is on the very tests that parked — or a lost wakeup in the mock's runtime. This file records
      that shape and the frequency; it does **not** claim the mechanism.

    For the next operator: do not add `#[ignore]`, do not "optimize" those families for speed, and
    do not read a parked lcov. The check that would name it is a fixture that logs acquire/release
    with the task id around `mock_agent_with_cleanup` / `TestHome::new` and re-runs the five families
    alone: a guard acquired by a task that never resumes is the signature.

## 6. The pending §359 question: `relay.rs` + `pairing.rs` + `control.rs`

**Answered by measurement: they were real gaps, not a classification artifact.**

| file | before | after | of which switch lane | of which waived | open |
|---|---:|---:|---:|---:|---:|
| `remote_host/pairing.rs` | 120 | 22 | 0 | 0 | 22 |
| `remote/relay.rs` | 124 | 4 | 0 | 4 | 0 |
| `remote/control.rs` | 138 | 45 | 0 | 16 | 29 |
| **total** | **382** | **71** | **0** | **20** | **51** |

The "has `mod tests`" heuristic happened to point at the right files for the wrong reason: the
lines were genuinely missed, and 270 of the 382 closed as soon as each file's protocol was driven
with a real fixture (round 5), then another 41 in round 7 (§1). What remains is not "the file is
untestable" — it is enumerated in §4: the client role's changing half, which needs a fixture that
this suite's own end-to-end drivers currently race (§4.1).

## 7. Gates run on this branch

- `cargo fmt -p future-host --check` — clean (the measurement is post-fmt).
- `cargo clippy -p future-host --all-targets -- -D warnings` — clean. The one fixup this round was
  `cargo fmt` reflowing the new fixtures' call sites; nothing after the measurement changed a line
  the report records.
- `cargo test -p future-host` — lib **1955 passed / 0 failed / 11 ignored**, twice (§1.1), plus
  two discarded passes accounted for in §1.1.
- `python3 scripts/coverage/lane-check.py docs/testing/coverage-waivers-2026-11/host-lane.json` —
  `ok: host/rust-lib at deb03ec4a — 2464 uncovered (2016 switch lane / 90 waived / 358 open)`,
  with the artifact recomputed (`lcov agrees`).
- The 7 new tests are: 4 in `remote::control::tests::serving` (invitation from a Desktop that is on,
  a removal through the supervisor, a takeover answered and one refused, the terminal switch stored
  both ways) and 3 loop/fixture tests (`spawn_host` answering and sweeping, `spawn_host` giving up on
  a `run` path it cannot create, `requests()` on an unreadable directory).

## 8. The lane's suite runtime, and why a 120 s check cannot pass

The lib suite takes **426-435 s** (this round's two measured passes). The 2× that used to sit on top of that
came from one fixture, and the measurement is worth keeping because every host verification pays
it:

- `test_support::tasks::task()` used to set every task's `cwd` to `std::env::temp_dir()`. A task's
  `cwd` becomes its conversation's workspace path, and the shadow-review before-snapshot measures a
  **non-git** workspace by walking it (`agent_bridge::review::capture_before` →
  `shadow_review::evaluate_volume`). So every task run walked the whole shared temp directory —
  measured at **36 s, 44 s and 54 s** for the *same single test* as TMPDIR filled up.
- `95a4299b0` gives each task an empty directory under a per-process root: that one test drops to
  **0.56 s**, `tasks::` to 4.7 s, and the whole lane to ~430 s. No assertion changed.

What is left is structural and deliberate: `test_support::agent::TestHome` **holds the
process-wide `TEST_HOME_LOCK` for its whole lifetime** (`host/src/test_support/agent.rs:686`,
because a test publishes `HOME`/`FUTURE_HOME` as real environment variables). Tests therefore run
essentially one at a time — 1955 tests × ~0.22 s ≈ 430 s.

**This serialization is the runtime, not the stall.** A pass that stops making progress does so at
0 % CPU with every fixture thread queued behind a holder that is not running — a different thing with
a different cause, recorded in §5.17. Nothing in this section is an optimization target for the
`approval`/`config`/`headless`/`import` families.

**So the machine check `cd .worktrees/cov-host && cargo test -p future-host --lib` exits 0 but needs
~7 minutes of budget.** A 120 s timeout reports a validator failure on a suite that is green: the
goal kernel runs a `--verify` command with `sh -c` and kills it after `DEFAULT_TIMEOUT` = **120 s**
(`orchestration/goal/src/validator.rs`), unless the orchestrator process itself has
`FUTURE_GOAL_VALIDATOR_TIMEOUT_SECS` set (`orchestration/goal/src/executor.rs`, `run_validator`).
On timeout the kernel records **`Inconclusive` — "validator failed to run"**, which says nothing
about the artifact.

### The gate this lane should carry instead

`scripts/coverage/lane-check.py` + `docs/testing/coverage-waivers-2026-11/host-lane.json` — **0.1 s**,
deterministic, and red when the numbers stop matching the tree. It ties three things together:

1. the lane record's buckets must add up (`switch + waived + open == lane total`);
2. the doc must quote the same total, the same waived/open counts and the measurement commit, so
   the prose cannot drift from the record;
3. when this machine still has the lcov artifact, per-line `DA:0` over the recorded scope is
   **recomputed** and must equal the record — a stale report, or a "complete but almost all zero"
   one written before the run finished, fails here instead of passing silently.

```bash
cd .worktrees/cov-host
python3 scripts/coverage/lane-check.py docs/testing/coverage-waivers-2026-11/host-lane.json
```

## 9. Round 8 — the client role's change half, the discovery pass, and a stack overflow that stops the full pass

Round 8 closes the two §4 rows this file called most specific: `remote/control.rs`'s client-role
changing half (**29 lines → 0**) and `agent_bridge/observer.rs`'s discovery/source arms (**28 of 29**;
the last one moves to §3, W18). **It does not move the lane's total, waived or open figures**, because
the whole-crate pass could not be taken on this machine: see "the blocker" below. The §1 numbers stay
the round-7 measurement until a full pass runs; what changed is recorded here, per line, from a
*focused* instrumented run.

### 9.1 What the focused run measures

`round8-focused.sh` cleans, rebuilds instrumented and runs the two families this round touched —
`remote::control` and `agent_bridge::observer` — then writes
`coverage/2026-11/after/host/round8-focused-lcov.info`. **120 passed / 0 failed in 13.95 s**, rc 0 for
both the run and the report.

`python3 coverage/2026-11/after/host/round8-lines.py round8-focused-lcov.info <file> <lines…>`
reads the per-line verdicts out of it:

| file | the §4 lines | result in the focused lcov |
|---|---|---|
| `host/src/remote/control.rs` | 939-941,945-946,951-952,960-966,970-975,981-987,997-998 (29) | **29 / 29 covered** |
| `host/src/agent_bridge/observer.rs` | 369-376,398-403,790-796,806-812 (28) | **28 / 28 covered** |
| `host/src/agent_bridge/observer.rs` | 1049 (1) | still `DA:0` — §3, W18 |

This is a *focused* run: it proves the tests reach those lines, not what the lane's whole-file totals
are. The lane record keeps round 7's `da0_total` / `waived` / `open` for that reason, and
`lane-check.py` still recomputes them from `round7-A-lcov.info`.

### 9.2 How the client role's change half was reached

Two things were in the way, and only one of them was about pairing.

1. **`suspended` outlived its fixture.** It is process-global, and the per-fixture reset
   (`remote::reset_after_shutdown_for_test`) only resets a server that was *closed*. The switch
   round's own suspend test clears `start_requested` before it ends, leaving `suspended` set for the
   rest of the process — so every later `start` took `start_generation`'s
   `!start_requested || suspended` arm and answered an **empty** status: no bridge, no invitation.
   The paired-machine fixture then failed on `host shows an invitation`, which names nothing about
   the pairing. `cleanup_host` now stops the host role before it resets it (the same rule the rest of
   that list already followed), and `start_host` reports the whole status instead of the missing
   field.
2. **The guard.** `remote::tests::runtime_tests::a_suspension_does_not_outlive_the_fixture_that_set_it`
   suspends a machine, drops its `HomeGuard`, acquires the next one and asserts `suspended` is clear
   and `start_requested` is false. Falsified once: with the `cleanup_host` drain removed the test
   fails at the second assertion, with it restored it passes.

With the machine drained, `remote::control::tests::serving::a_client_role_change_runs_against_the_paired_machine`
drives all four operations of `perform_peer` against a real host bridge on the in-process broker —
connect (the answer reads the list back), takeover (the host answers `changed: false` for a machine
it already drives, and a silent field is a result, not a refusal), disconnect (the pairing survives)
and remove (with the platform scripted to refuse the revoke: the row is gone locally and the note is
the only place the operator learns the cleanup is owed).

### 9.3 The discovery pass and the observer's source arms

`agent_bridge/observer.rs`'s four silent arms are driven through one tick of the real
`spawn_session_discovery` loop, with the real store and a scripted Agent:

| test | arm | what it prints, and therefore what it asserts |
|---|---|---|
| `a_discovery_tick_releases_a_deletion_intent_no_client_can_deliver` | 369-374 | `released 1 undeliverable deletion intent(s)` |
| `a_missing_delete_outbox_is_reported_and_the_loop_keeps_ticking` | 375-376 | `undeliverable delete reconcile failed: no such table: agent_delete_outbox` |
| `a_discovery_tick_removes_a_conversation_the_agent_confirmed_deleted` | 398-403 | `removed 1 conversation(s) deleted outside the GUI` |

Each fixture is scripted so the loop's own question is the only one the Agent is asked (one presence
answer shape, no ordering): the release test has no mirrored conversation at all, and the deletion
test has no fence. The first attempt put both in one tick and failed — `session_presence` requires
the answer to name *exactly* the ids asked for, and the two passes ask about different batches.

The observer's attach loop gets 790-796 and 806-812:

- `run_observer_defers_an_attach_whose_source_it_cannot_verify` scripts a `deleted` presence answer,
  so the source check refuses, the observer names the check, backs off and re-attaches; the
  assertion is that it asks at least twice and that **nothing is replayed before the source is
  verified**.
- `run_observer_resets_its_cursors_when_the_source_changes` re-attaches onto a new restore generation
  of the *same* database (a different database id is refused one layer earlier, by the same source
  check — measured, not assumed) and asserts the run binding is gone. Falsified once: with the
  generation unchanged the assertion fails, because the first attach's `agent_start` is what sets the
  flag. The second cycle's stream is `StreamScript::Hang`, so the loop parks there instead of
  starting a third attach and re-setting it.

### 9.4 The blocker: the full `--lib` pass overflows a tokio worker's stack

A whole-crate pass cannot be completed on this machine right now, so §1's totals stand:

| run | filter | result |
|---|---|---|
| `round8-full-lib-run.log` | none | parked at 0 % CPU in `agent_bridge::import`, 23 threads queued on `TEST_HOME_LOCK` and 7 on `MOCK_LOCK`, no holder running (`sample` → `park-holders-round8.py`); discarded, never read |
| `round8-remote-module.log` | `remote::` | `thread 'tokio-rt-worker' has overflowed its stack` |
| `round8-verify-e2e.log` | `remote::verify_e2e` | same, at `the_baseline_monitor_reading_runs_before_the_traffic` |
| `round8-verify-e2e-baseline.log` | same, **this round's changes stashed** | same overflow → **not this round's** |
| `round8-remote-peer-baseline.log` | `remote_peer::`, changes stashed | same overflow → **not this round's** |

The stack overflow is reproducible on a tree with this round's work stashed and reproduces under the
env round 7 used (`HOME` at the worktree's `target/test-home`, `ulimit -n 8192`), so it is not an fd
limit and not a fixture this round added. It is the reason the lane reads round 7's numbers: a pass
that dies mid-run has no verdict to report, and a report written from partial profraw is exactly the
"complete but almost all zero" shape §7.3 warns about.
