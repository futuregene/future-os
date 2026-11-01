# host (Rust, `cargo test -p future-host --lib`) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The llvm-cov
summary's `Lines` column is not the acceptance number (it adds ghost lines).

**Lane** — `future-host`'s **lib** unit tests only. The integration-test binaries and the
`#[ignore]`d real-network legs are measured by their own lanes; this file is about the lib.

**Last measurement**: `test/cov-host` at `b4b8f605e`, rustc **1.99.1** (`rust-toolchain.toml`),
`HOME=target/test-home`, `ulimit -n 8192`. Artifacts (gitignored) are in
`coverage/2026-11/after/host/`: `round5-A-lcov.info` (pass A, the numbers quoted below),
`round5-B2-lcov.info` (an independent repeat, §1.1), `round5-run.log` / `round5-B2-run.log`, the
per-line inventories `round5-A-missed.json` / `round5-B2-missed.json`, the bucket arithmetic in
`buckets5.py`, the per-file deltas in `delta5.py` / `perfile5.py`, the §4 rows in
`open-ranges5.py`, and the run-to-run line diff in `flaky-lines.py`. Superseded by this round:
`base3-lcov.info` (2589, taken right after the rebase onto `dev-huichen`), `final-lcov.info`
(2589, the previous round's pass A) and `repro2-*`.

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

The run is also the gate: **1931 passed, 0 failed, 11 ignored** in 414.00 s, then `report rc=0`
(`round5-run.log`).

## 1. Before / after, and where every remaining line sits

Both columns are `DA:0` over `host/src/`, same lane, same recipe. "before" is
`base3-lcov.info` — the baseline taken after this branch was rebased onto `dev-huichen`, which is
the tree this round's work started from (it reproduced the previous round's 2589 exactly).

| | before (`base3-lcov.info`) | after (`round5-A-lcov.info`) | Δ |
|---|---:|---:|---:|
| **lane total** (`host/src/**`) | 2589 | **2553** | **−36** |
| switch-lane files (§2.1) | 1884 | 1884 | 0 |
| **inside in-scope files, switch lane only** (§2.2, §2.3) | 125 | **132** | **+7** |
| **in-scope** (lane total − §2.1 files − §2.2/§2.3 lines) | 580 | **537** | **−43** |

Every one of the 2553 remaining lines is in exactly one bucket (`buckets5.py` prints this; the
four numbers sum to the total):

| bucket | lines | what it is |
|---|---:|---|
| **switch lane: their own files** | **1884** | §2.1 — five files only the network legs execute |
| **switch lane: inside in-scope files** | **132** | §2.2 (125: `tasks.rs`'s two `#[ignore]`d tests, 83; `remote/test_support.rs`'s `reconnect_secure_channel`, 42) plus §2.3 (7: the `fixture_mode()` arms that `cfg!(test)` makes constant) |
| **waived** | **74** | §3 rows W1-W7 (21) and W9-W12 (33), plus the closing-edge attribution W8 (20) |
| **open** | **463** | §4 — real gaps. **Not waived.** |

So the campaign figure for this lane today is **463 open**, not 0: this pass closed 36 lines and
added 12 tests (1919 → 1931), and 2016 of the residual are another lane's to measure.

`buckets5.py` asserts, before it counts, that every §3 waiver line and every §2.3 fixture arm is
still `DA:0` — a waiver this measurement would have falsified, or an arm it would have covered,
fails the run instead of being silently retired. All 74 waived lines and all 7 arms held.

Where this pass's 36 closed lines came from (per file, before → after, `perfile5.py`; a row whose
count grew is a line number that moved with the edit, not a regression):

| file | before | after | what closed it |
|---|---:|---:|---|
| `remote_host/business/mod.rs` | 11 | **0** | `a8d871797` — the capability gate's two refusals (`UPGRADE_REQUIRED` instead of the page, and a preflight that cannot be read reported as "I could not check" rather than "compatible"), plus the two sides that prove the gate is a gate: a legacy session still reaches the handler and a command outside the list never pays the preflight |
| `store/test_support.rs` | 5 | **0** | `5859070e1` — the schema connection the entry fixtures open reads the same row the store writes |
| `tasks/service.rs` | 5 | 1 | `5859070e1` — the task store's root with no `FUTURE_HOME`: the application directory's parent, not the empty temp fallback |
| `events.rs` | 5 | 2 | `b4b8f605e` — `emit_control_claimed` publishes through the presentation gate and does not attach one |
| `remote_host/mod.rs` | 3 | **0** | `b4b8f605e` — the host's own claim hook, same invariant |
| `remote_peer/host.rs` | 5 | 2 | `5859070e1` — `convert` keeps `Json`/`RemoteTransport`/`RemoteAuthorization`/`Remote` and wraps a host-only error still downcastable |
| `runtime.rs` | 7 | 4 | `929a4a798` — `tasks_frozen` answers for the registry the shutdown paths ask about |
| `startup.rs` | 7 | 4 | `929a4a798` — the default tracker is a launch at its first step, which `ready()` ends |
| `store/threads.rs` | 3 | 1 | `ae228f549` — a blank title is not a seed for either the importer or the healing pass |
| `scheduler.rs` | 2 | 3 | `ae228f549` — the three global jobs are built on the configured intervals (`31`); `285`/`290`/`438` are earlier asserts' message arguments, moved by the insert |
| `shutdown/services.rs` | 17 | 16 | no test: `20` (`return Err(error.into())`) is one of the two lines §1.1's previous pair named as race-dependent; it read covered in both passes this round |
| `store/migrations.rs` | 16 | 17 | none: `78` (a fresh database whose ledger stops part-way) read covered in the baseline and reads 0 in both passes now — the same race-attributed class as §1.1 |

Earlier passes, for the record: `agent_bridge/queries.rs` 12 → 0, `remote/mod.rs` 14 → 0,
`agent_host.rs` 58 → 28, `agent_bridge/run_control.rs` 14 → 2 and `agent_bridge/stream.rs` 14 → 4
(`188e08161`); `agent_bridge/reconciliation.rs` 39 → 2, `application/threads.rs` 15 → 3 and
`agent_client.rs` 16 → 7 (`f5b44775a`); `store/reconcile.rs` 13 → 4 and
`agent_bridge/observer.rs` 40 → 39 (`5b88f9f9d`); `agent_bridge/queries.rs` 69 → 12,
`remote/control.rs` 138 → 88, `remote_host/pairing.rs` 120 → 22 and `remote/relay.rs` 124 → 4
(`75946a7ed`); `remote_platform.rs` 18 → 10 and `agent_bridge/config.rs` 24 → 4 (`ed6547572`);
`application/threads.rs` 40 → 22, `application/runs.rs` 14 → 0, `application/skills.rs` 26 → 3 and
`test_support/timing.rs` 40 → 1 (`0b9c945c3`).

### 1.1 Same-number reproduction: the measurement, run twice

Pass B (`round5-B2.sh`) runs the identical recipe on the same instrumented build with its
`.profraw` files deleted first, into an independent lcov file. Both passes are green:
**1931 passed / 0 failed / 11 ignored** in 414.00 s (A) and 435.76 s (B).

| | pass A (`round5-A-lcov.info`) | pass B (`round5-B2-lcov.info`) |
|---|---:|---:|
| lane total `DA:0` | **2553** | **2553** |
| switch-lane files | 1884 | 1884 |
| in-scope | 537 | 537 |
| lines that differ between the runs | — | **0** |

`flaky-lines.py` (which prints the source of every line that differs) printed **nothing**: this
pair agreed line for line, which is the strongest form this lane's measurement has taken. It is
not the general case and this file does not claim it is: the previous round's pair moved by two
lines (`agent_host.rs:502`, `shutdown/services.rs:20`, both named there), and the round before by
three, all in `agent_host.rs`. What this round *does* add to that picture is that a line can also
flip between *passes of different rounds* — `store/migrations.rs:78` reads 0 in both passes now and
read covered in the baseline. So the honest claim stays: **the measurement is stable to a couple of
named lines, and every line that moves is named.**

One pass in this round was **discarded rather than read**: the first B attempt was
`SIGKILL`ed mid-suite (`round5-run.log`: `signal: 9, SIGKILL`), and its lcov is kept only as
`round5-B-DISCARDED-sigkill.info`. A pass that was killed leaves partial `.profraw` files that
look like a measurement — the earlier round's `after-lcov.info` came from exactly that mistake
(§5.2). The same rule applied to the pass before it: `round4-A-DISCARDED-two-failures.info` came
from a run in which two of this round's new tests failed, so it was thrown away and the tests
fixed before measuring again.

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

## 3. Waived constructs — 74 lines

Each row names the mechanism, the test that pins the neighbouring covered lines (the positive
control), and a falsifier. A waiver that cannot name its falsifier is not evidence
(`docs/testing/waiver-ledger.md`). `W1`–`W8` are the classes defined in
`.future/memory/coverage-llvm-cov.md`: W1 cfg/​platform · W2 defensive dead arm · W3 OS failure
injection · W4 race window · W5 attribution artifact · W6 ghost line · W7 mock closure ·
W8 dead producer/deliberately-unexecuted line.

| # | file:line | lines | class | construct | positive control | falsifier |
|---|---|---:|---|---|---|---|
| W1 | `host/src/remote/relay.rs:469,587,594,597` | 4 | W2 | the test fixture's own failure arms: `Ok(_) => panic!("{what}")` in `a_refused_grant`, the mock relay's socket-read `expect`, and the `continue` for a buffer that never held a request head | `remote::relay::tests::register_signs_the_room_request_and_mints_a_grant` and `…::a_relay_that_is_not_listening_is_a_transport_failure` run the same fixture on its success and error paths | hand the helper an `Ok(_)`, or open a connection that writes nothing: 469 and 597 are taken |
| W2 | `host/src/remote/control.rs:2134` | 1 | W2 | `Ok(_) => panic!("{what}")` in `a_refused_peer_request` | `remote::control::tests::perform_peer_reads_the_list_and_asks_which_machine` uses the same helper on its `Err` arm | call it with an `Ok(PeerOutcome)` — the suite fails there |
| W3 | `host/src/remote/trust.rs:96-98` | 3 | W6 | `#[cfg(not(test))] fn bundle_path()`: the body has no executing caller in a `--lib` build; its `DA:0` record comes from the crate's second (dependency) compilation, the "lib 双编译合并" shape in the memory note | `bundle_at`'s own tests cover the same snapshot-and-fallback logic | an integration-test binary calling it — another lane |
| W4 | `host/src/remote/test_support.rs:784,789` | 2 | W1/W6 | the `else` arms of `cfg!(target_os = "macos")` in the mock's `probe_sandbox` answer (`"unsupported_platform"`, `"none"`) — this machine is macOS, so the other platform's text never executes | `agent_bridge::queries::probe_tests::the_sandbox_probes_ask_the_agent_and_map_its_answer` covers the same answer's mapping | run the suite on Linux: those two lines execute and the macOS ones read 0 |
| W5 | `host/src/remote/test_support.rs:801-808` | 8 | W8 | the mock's default `probe_thinking_level` answer — evaluated only when a test leaves the command unscripted | `agent_bridge::config::tests::a_thinking_level_probe_forwards_the_request_and_keeps_a_refusal_as_an_answer` owns that command via a script, so the default is bypassed on purpose | drop one `probe_thinking_level` script from any test: the default arm runs |
| W6 | `host/src/remote/test_support.rs:356-358` | 3 | W8 | `RecordingSink::is_empty` / `len` — accessors no handler test currently calls | the same type's `push`/`drain` are used by the handler tests | one assertion in any handler test covers all three |
| W7 | `host/src/remote/test_support.rs:1963,2193,2710` | 3 | W2 | failure arms inside tests: a `violations.push(…)` for a doc-contract breach, its `join` in the assert message, and `Err(error) => panic!(…)` in a deferred-arm matcher | the same tests pass, so their happy paths execute | break the doc contract the test checks, or answer the deferred call with a failure |
| W8 | 20 lines, listed in `coverage/2026-11/after/host/closing-edges.txt` (`store/legacy_history.rs:24,81,212,260,265`, `store/migrations.rs:39,76,139`, `agent_bridge/stream.rs:318,388`, `agent_bridge/import.rs:632`, `agent_bridge/presence.rs:112`, `application/threads.rs:420`, `store/cleanup.rs:456`, `store/deletions.rs:26`, `store/managed_cleanup.rs:28,38`, `store/review_snapshots.rs:203`, `store/runs.rs:184`, `store/approval_assessments.rs:63`) | 20 | W6 | a line that holds **nothing but** a call's closing error edge (`)?;`, `})?;`, `)?`) — "多行调用把 `)?;` 单独落一行 → 该行只有错误臂 → `DA:0`" (`.future/memory/coverage-llvm-cov.md`) | the statement itself ran: the line directly above each of these 20 reads `DA>0` (checked mechanically by `buckets5.py`). The closing-edge lines whose neighbour is *also* uncovered stay in §4 (`agent_bridge/config.rs:202`, `agent_client.rs:178,209`, `remote/web_server.rs:61`, `remote_host/pairing.rs:293`, `remote_platform.rs:53`, `remote/test_support.rs:67`) | put one of these calls on a single line (shrink its argument list into rustfmt's 60-column `fn_call_width`) and the line disappears from the next report with no test change; conversely a caller that actually forces the error edge — a closed store, an unreadable file — covers it |
| W9 | `host/src/remote_host/business/catalog.rs:104-106,122-130` | 11 | W2 | the two "the fork produced nothing usable" arms of the phone's `fork_session`, plus the `get_thread` failure beside them. All three sit behind one invariant: `agent_bridge::fork_agent_session` refuses a fork whose session id came back empty (`agent_bridge/session.rs:316-318`) and *creates* the thread itself, with that session id bound, before returning its id (`session.rs:373-384`, `get_or_create_thread_for_agent_session(… agent_session_id: Some(new_session_id))`) | `business::catalog::dispatch_tests::fork_session_creates_and_reports_the_child_conversation` and `…::a_refused_fork_is_reported_with_the_agents_reason` drive the same call on the success path and the refusal path, and both read the child back through the same `get_thread` | make `fork_agent_session` return an id it did not persist (a session the Agent forked on another machine, or a fork that returns the parent's id) — then 122-128 answers and all eleven lines execute |
| W10 | `host/src/remote_host/business/settings.rs:148-150,177-181` | 8 | W1 | the sandbox probe's non-answer arms: `product_sandbox_status()`'s `Err` (148-150, 179-181) and `product_sandbox_available()`'s `Ok(false)` (177-178). On macOS that function is `#[cfg(target_os = "macos")] { Ok((true, None)) }` (`settings.rs:301-310`): the sandbox is a constant there, so neither the `Err` nor a `false` availability has an input in this lane | `business::settings` tests drive `get_settings` and the settings write through `product_sandbox_available()`, so the macOS arm itself is covered | run the lane on Linux or Windows, where the answer comes from `agent_bridge::probe_sandbox()`: a probe failure reaches 148-150 and 179-181, and a host without a sandbox reaches 177-178 |
| W11 | `host/src/shutdown.rs:186-187,204-208` | 7 | W3 | the two `std::thread::Builder::new().spawn(…)` failures of the stop path: the cleanup watchdog (186-187) and the cleanup worker (204-208). Thread creation fails only when the OS refuses it | `shutdown`'s tests run the same `stop()` to completion, including the worker's finish/progress reporting that the arms surround | exhaust the process's thread budget (lower `RLIMIT_NPROC`/`ulimit -u` to what is already in use) and call `stop()`: `pthread_create` fails, `spawn` returns `Err`, and both arms answer |
| W12 | `host/src/remote_host/business/providers.rs:58-64` | 7 | W2 | `serde_json::to_value(view)` in `reply_view`, where `view: agent_providers::ProvidersView` is `Vec<BuiltinProvider>` + `Vec<CustomProvider>` of `String`/`bool`/`usize` fields (`agent_providers/mod.rs:59-76`). `to_value` fails only on a non-string map key or a non-finite float, and neither type occurs in the view | every provider test answers through `reply_view`, so the same `to_value` succeeds on the same types | add a field whose `Serialize` can fail (an `f64` that can be NaN, or a map keyed by a struct) — the arm then has an input and answers |

Waived total: **74** (21 in rows W1-W7, 20 in W8, 33 in W9-W12). Nothing else in §4 is claimed as
waived.

## 4. Open residual — 463 lines in 65 files

**Not waived.** Every row is the measurement plus whatever has actually been read; "not yet read"
means exactly that. Rows are `DA:0` line numbers in the tree this file records (`b4b8f605e`).

| file | open lines | `DA:0` | mechanism |
|---|---:|---|---|
| `remote/control.rs` | 87 | 580-586,589-590,592,594-595,602,606-609,612-613,615,617-618,623,743,748,767,776-777,792,815,825,831-837,839,847-851,854-856,939-941,945-946,951-952,960-966,970-975,981-987,989-993,997-998,1237-1242 | `spawn_host`'s poll loop (its only caller is `desktop/src-tauri/src/lib.rs:1422`), the peer-op branch of `serve_one`, and the `perform`/`perform_peer` arms that need a running supervisor (`pair_new_device`, `unpair_slot`, `takeover`, `enable`, `disable`, peer connect/disconnect/remove) |
| `agent_bridge/observer.rs` | 39 | 369-376,398-403,790-796,806-812,1040-1041,1043-1045,1047-1049,1108,1115,1234 | reconcile result logs, source-verification backoff, required-content hydration failures, and two `return false` arms in the projection path |
| `agent_host.rs` | 28 | 144,173-174,177,216-217,311-313,334,363,435,443,451,487,503,531,539,665,762-764,766-767,815,890,1027-1028 | `is_stopping()` arms (144, 177 — closing the process-wide admission is one-way for the test binary), the recovery closure's unfinished-retirement arm (173-174), the handle-level idle admission (216-217, needs a live `AgentHandle`), worker-spawn failure (311-313), `Stopped` with no yield reason (334), the external-mode validation (363), attach-only refusals (435, 443, 451), the stop/serve precedence arms (487, 503, 531, 539, 665), and the fixture harness's own failure arms (762-767, 815, 890, 1027-1028) |
| `shadow_review/snapshot.rs` | 24 | 81-91,234-245,266 | the `stage()` retry that needs `git add` to fail once and a path to become ignored on the retry, which no fixture produces yet |
| `remote_host/pairing.rs` | 22 | 292-293,372,490-492,503-505,540-541,770-776,831-833,1682 | leftover refusal arms around the platform calls |
| `cli_tool.rs` | 19 | 84,93,100,105,255-257,326-328,333,345,385-387,402,705,732,832 | the `cfg(windows)` executable name (84, W1-shaped but left open: the falsifier is a Windows lane, not a waiver), the `"bundled"`/`"development"` source labels (93, 100 — the second is produced only by the `cfg(not(test))` candidate that this lane compiles out), the 5 s failed-probe cache renewal (105), the deadline-free constructor no test calls (255-257), the probe's own error and post-exit limit arms (326-328, 333, 345 — reachable only through a `try_wait` error or a race between the size check and the child's exit), the discovery-failure path (385-387), a closing brace (402), and two fixture asserts' message arguments (705, 732, 832) |
| `remote/trust.rs` | 17 | 64,66,68,70-71,73,75,87,91-93,169-174 | `bundle_with_extra`'s arms behind a `OnceLock` (64-93 — one closure execution per process, so at most one arm per run), and `load_platform_certs`'s error collection (169-174) |
| `shutdown/services.rs` | 16 | 47-49,68,70-71,73,75-76,81,93,96,98-99,280,321 | service shutdown error arms and two closing braces |
| `tasks.rs` | 15 | 126-128,228-230,234,328,439,476,482,572-573,1859,2176 | store-open / delete / thread-load failure logs |
| `store/migrations.rs` | 14 | 57-59,78,126-127,163,203-206,222-224 | migration failure/repair branches (a broken schema, a missing foreign-key target, a missing binding index) |
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
| `remote/supervisor/start.rs` | 3 | 33,38-39 | the fixture-mode branch of `start_once` (`remote/supervisor/start.rs:12,17-18` were the same class before this round's re-measurement) |
| `remote/secure.rs` | 3 | 39-41 | not yet read |
| `instance.rs` | 3 | 24,53,80 | the no-override identifier (24), and two `directory.parent()` guards that need a path with no parent |
| `files.rs` | 3 | 637-639 | not yet read |
| `application/threads.rs` | 3 | 117,147,421 | `update_thread_*`'s agent-propagation block closers (117, 147: the brace after the propagation call, reached only when the call is skipped) and one brace behind a `?` |
| `application/skills.rs` | 3 | 48-50 | the `get_skill_guide` delegation (needs the platform HTTP fixture) |
| `store/workspaces.rs` | 2 | 60,914 | not yet read |
| `shadow_review/maintenance.rs` | 2 | 69,100 | the `}` after a failed consistency check and the same for interrupted-run recovery |
| `remote_peer/transfer.rs` | 2 | 45-46 | not yet read |
| `remote_host/command_diagnostics.rs` | 2 | 71,136 | a binary reply's byte count and the `replied=none` branch (a command the sink was never told the outcome of) |
| `remote_host/business/history.rs` | 2 | 305,325 | the `get_event_content_range` dispatch arm and its failure paths |
| `remote/web_server.rs` | 2 | 60-61 | the `local_addr()` failure arm |
| `events.rs` | 2 | 203,226 | two `panic!` arms inside tests (a `let … else` that the passing test never takes) |
| `environment.rs` | 2 | 67,125 | the previous-log preservation failure report, and `File::open` failing for a reason other than `NotFound` |
| `agent_bridge/stream.rs` | 2 | 476,1539 | one `break` in the collector's re-attach loop (476) and a test's `panic!` for a fold that must end (1539) |
| `agent_bridge/run_control.rs` | 2 | 141-142 | two closing braces after a block that ends in `return` (141-142) — attribution, not a statement |
| `agent_bridge/reconciliation.rs` | 2 | 488-489 | the two block-closing braces after `settle_from_agent_terminal` `return`s: reached only when the read-back names another active run and carries no `requestedRun` |
| `agent_bridge/prompt.rs` | 2 | 235,258 | a conversation with preserved history but no writable session, and a recovery archive whose entries are all non-conversational |
| `workspaces.rs` | 1 | 64 | `delete_workspace`'s empty thread list — the `has_presentation()` false arm, which is why it reads 0 only when no test has attached a sink yet (order-dependent, `agent_bridge/observer.rs:2501` attaches one) |
| `test_support/timing.rs` | 1 | 173 | not yet read |
| `tasks/service.rs` | 1 | 599 | an assert's message argument in the new `without_an_override` test |
| `store/threads.rs` | 1 | 1899 | a test's `remove_dir_all` guard, reached only when the generated directory already exists |
| `store/approval_assessments.rs` | 1 | 172 | not yet read |
| `remote_peer/runtime.rs` | 1 | 355 | not yet read |
| `remote_peer/host.rs` | 1 | 158 | the assert-message argument of the new `convert` test |
| `remote_host/session_files.rs` | 1 | 74 | a `metadata()` failure between listing and stat |
| `remote_host/business/transfers.rs` | 1 | 59 | not yet read |
| `error.rs` | 1 | 106 | a remote-client error → `host::Error` conversion arm |
| `build_info.rs` | 1 | 171 | not yet read |
| `auth_store.rs` | 1 | 269 | not yet read |
| `agent_bridge/presence.rs` | 1 | 86 | not yet read |

Highest-yield next steps, from the table: `shadow_review/snapshot.rs` (24 — a git fixture where
`git add` fails once), `agent_bridge/observer.rs`'s projection `return false` arms (39 — a crafted
snapshot event), `store/migrations.rs`'s repair branches (14 — a deliberately broken schema), the
group of seven-line files that are only "not yet read" (`business/prompt.rs`,
`agent_bridge/headless.rs`, `shadow_review/repository.rs`), and `remote/control.rs`'s `spawn_host`
block (38, which needs either a supervisor fixture or a decision to leave it open — §5.7).

### What needs a product seam (judged OPEN, not waived)

Four blocks cannot be measured without changing product code to make themselves observable. They
stay open and the missing piece is named here rather than waived:

- `remote_host/business/prompt.rs:148-154` — the `accepted_rx.await` `Err` arm. Needs a way for a
  test to drop the acknowledgement sender; today it is an internal `oneshot` created inside the
  handler, so no fixture can reach it.
- `remote/trust.rs:64-93` — `bundle_with_extra`'s four arms inside one `OnceLock` closure, of which
  at most one runs per process. Needs the body extracted from the closure, which is product code
  written for the measurement (§5.8).
- `remote/control.rs:580-986` — `spawn_host` and the supervisor-backed `perform` arms. Needs a
  supervisor fixture (a live host bridge), not a seam: the loop's only caller is
  `desktop/src-tauri/src/lib.rs:1422`.
- `agent_bridge/observer.rs:1040-1049` — the projection's `return false` arms. Needs a crafted
  snapshot event whose shape no fixture produces yet; this one is a fixture, not a seam.

## 5. Rejected approaches (this lane)

1. **Classifying a file by "it has `mod tests`".** The earlier pass's method for 359 lines;
   measurement settled it (§6).
2. **Trusting an `after-lcov.info` whose run was killed or failed.** The first measurement of this
   campaign merged a corrupt profraw (`warning: … invalid file header` / `error: no profile can be
   merged`). This round added two more instances of the same trap and threw both away instead of
   reading them: `round4-A-DISCARDED-two-failures.info` (a green-looking lcov from a run in which
   two new tests failed at their first assertion, so the lines after it read 0) and
   `round5-B-DISCARDED-sigkill.info` (partial profraws from a `SIGKILL`). Both stayed on disk,
   renamed, so the discard is auditable.
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
   behind for every later test in the binary. Left open instead.
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
    Both passes run detached, each with its own log.
12. **Writing the capability-gate tests against the bridge mock and a `TestHome`.** That is what the
    first attempt of this round did, and it produced two failures that are worth separating, because
    only one of them is proven:
    - **Proven, deterministic**: a bridge-mock test (`crate::test_support::agent::mock_agent`) and a
      remote-mock test (`remote::test_support::ensure_mock_agent`) must not run in the same module.
      The two mocks each re-point the process-wide `FUTURE_AGENT_GRPC_ADDR` at *their own* server
      (`test_support/agent.rs:400-410`), so a test in one family can have its command answered by
      the other's default script: four of the module's 67 tests failed with "handler unexpectedly
      succeeded" and "the page must not be served to this peer" — i.e. a scripted refusal was never
      honoured — and every one of them passed alone. Rewriting them onto the module's own family
      (`mock_agent_lock` + `HomeGuard` + `ensure_mock_agent` + `script_for`) made the module green
      in every run since, and removed the cross-family window entirely.
    - **Not proven**: the first *full-suite* run with those bridge-mock tests in this module
      deadlocked — every test thread parked at `test_support/agent.rs:423` (the bridge mock's lock)
      at 0 % CPU for 20 minutes, with the mock's runtime idle. `sample` named the park but not the
      cycle, and the machine was carrying two other sessions' suites (load average 46-56). The
      rewrite removed the condition; the two green full passes do not prove which lock pair cycled,
      so this file does not claim a mechanism it cannot show. What it does claim is the rule: one
      mock family per module.
13. **Asserting that a lib test binary has no presentation sink.** `events::attach` is a
    process-global `OnceLock` and `agent_bridge/observer.rs:2501` attaches one, so
    `has_presentation()` depends on which test ran first. The first version of the two claim-hook
    tests asserted it was false and failed in the measured run. They now assert the invariant that
    holds either way: raising the claim does not change it.

## 6. The pending §359 question: `relay.rs` + `pairing.rs` + `control.rs`

**Answered by measurement: they were real gaps, not a classification artifact.**

| file | before | after | of which switch lane | of which waived | open |
|---|---:|---:|---:|---:|---:|
| `remote_host/pairing.rs` | 120 | 22 | 0 | 0 | 22 |
| `remote/relay.rs` | 124 | 4 | 0 | 4 | 0 |
| `remote/control.rs` | 138 | 87 | 0 | 1 | 87 |
| **total** | **382** | **113** | **0** | **5** | **109** |

The "has `mod tests`" heuristic happened to point at the right files for the wrong reason: the
lines were genuinely missed, and 266 of the 382 closed as soon as each file's protocol was driven
with a real fixture. What remains is not "the file is untestable" — it is enumerated in §4, and
most of it needs a running remote supervisor.

## 7. Gates run on this branch

- `cargo fmt -p future-host --check` — clean (the measurement is post-fmt).
- `cargo clippy -p future-host --all-targets -- -D warnings` — clean. One change landed *after*
  the measurement to get there: `#![allow(clippy::await_holding_lock)]` on the `business::tests`
  module, because the capability tests hold the mock's lock across the awaits that consume its
  script (the same allowance `business::catalog` and `business::settings` already carry per test).
  That attribute shifts three lines inside `remote_host/business/mod.rs`, a file with **no `DA:0`
  record at all**, so no number in this file moves; the record's `measurement_commit` stays the
  measured tree.
- `cargo test -p future-host` — lib **1931 passed / 0 failed / 11 ignored**, twice (§1.1).
- `python3 scripts/coverage/lane-check.py docs/testing/coverage-waivers-2026-11/host-lane.json` —
  `ok: host/rust-lib at b4b8f605e — 2553 uncovered (2016 switch lane / 74 waived / 463 open)`,
  with the artifact recomputed (`lcov agrees`).

## 8. The lane's suite runtime, and why a 120 s check cannot pass

The lib suite takes **414-436 s** (this round's two passes). The 2× that used to sit on top of that
came from one fixture, and the measurement is worth keeping because every host verification pays
it:

- `test_support::tasks::task()` used to set every task's `cwd` to `std::env::temp_dir()`. A task's
  `cwd` becomes its conversation's workspace path, and the shadow-review before-snapshot measures a
  **non-git** workspace by walking it (`agent_bridge::review::capture_before` →
  `shadow_review::evaluate_volume`). So every task run walked the whole shared temp directory —
  measured at **36 s, 44 s and 54 s** for the *same single test* as TMPDIR filled up.
- `95a4299b0` gives each task an empty directory under a per-process root: that one test drops to
  **0.56 s**, `tasks::` to 4.7 s, and the whole lane to ~435 s. No assertion changed.

What is left is structural and deliberate: `test_support::agent::TestHome` **holds the
process-wide `TEST_HOME_LOCK` for its whole lifetime** (`host/src/test_support/agent.rs:686`,
because a test publishes `HOME`/`FUTURE_HOME` as real environment variables). Tests therefore run
essentially one at a time — 1931 tests × ~0.22 s ≈ 430 s.

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
