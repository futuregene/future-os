# host (Rust, `cargo test -p future-host --lib`) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The llvm-cov
summary's `Lines` column is not the acceptance number (it adds ghost lines).

**Lane** — `future-host`'s **lib** unit tests only. The integration-test binaries and the
`#[ignore]`d real-network legs are measured by their own lanes; this file is about the lib.

**Last measurement**: `test/cov-host` at `41cf889bf` (cut from `origin/dev-huichen@12b89ee7e`),
rustc **1.99.1** (`rust-toolchain.toml`), `HOME=target/test-home`, `ulimit -n 8192`.
Artifacts (gitignored) are in `coverage/2026-11/after/host/`: `final-lcov.info`, `final-run.log`,
`final-missed.json`, and the bucket arithmetic in `buckets.py`.

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

The run is also the gate: **1883 passed, 0 failed, 11 ignored** in 437 s, then `report rc=0`
(`final-run.log`).

## 1. Before / after, and where every remaining line sits

Both columns are `DA:0` over `host/src/`, same lane, same recipe.

| | before (`before-lcov.info`) | after (`final-lcov.info`) | Δ |
|---|---:|---:|---:|
| **lane total** (`host/src/**`) | 3265 | **2822** | **−443** |
| switch-lane files (§2.1) | 1884 | 1884 | 0 |
| **in-scope** (lane total − switch-lane files) | 1381 | **938** | **−443** |

Every one of the 2822 remaining lines is in exactly one bucket (`buckets.py` prints this; the three
numbers sum to the total):

| bucket | lines | what it is |
|---|---:|---|
| **switch lane** | **2009** | covered only by legs this lane does not run: the five files below (1884) plus 125 lines inside in-scope files that only those legs execute (`tasks.rs`'s two `#[ignore]`d tests, 83; `remote/test_support.rs`'s `reconnect_secure_channel`, 42) |
| **waived** | **24** | §3 — each with a mechanism, a positive control and a falsifier |
| **open** | **789** | §4 — real gaps. **Not waived.** |

So the campaign figure for this lane today is **789 open**, not 0: this pass closed 443 lines and
waived 24, and 2009 of the residual are another lane's to measure.

Two lines moved in the *opposite* direction, and they are worth naming rather than smoothing over:
`agent_host.rs:502` and `shutdown/services.rs:319-320` are arms **inside tests** (a `select!`
branch and a task parked on `pending()`), so which of them runs depends on how that run's races
landed. They read as uncovered here because this measurement is the one this file pins. A third
pair, `startup.rs`, moved only because this pass added 28 lines to its test module (§4).

Where the 443 closed lines came from (per file, before → after):

| file | before | after | commit |
|---|---:|---:|---|
| `remote/relay.rs` | 124 | **6** | `75946a7ed` — register / refresh / revoke against a loopback relay, each request's signature re-derived from the bytes that arrived |
| `remote/control.rs` | 138 | **88** | `75946a7ed` — `serve_one` against real request files (claim, answer, unreadable, unclaimable), `perform`/`perform_peer` refusals |
| `remote_host/pairing.rs` | 120 | **22** | `75946a7ed` — a self-hosted invitation minted against a relay room; rotation, re-registration, room deletion |
| `agent_bridge/queries.rs` | 69 | **13** | `ed6547572` — sandbox probes, Windows reset, the content page for an exact run, run-snapshot fallbacks |
| `remote_platform.rs` | 18 | **10** | `ed6547572` — a `--relay-ca` that is missing, holds no certificate, or is handed to the backend |
| `application/threads.rs` | 40 | **22** | `0b9c945c3` — list / open / recency reads, pin, archive, restore |
| `application/runs.rs` | 14 | **0** | `0b9c945c3` — the four run reads a window makes |
| `application/skills.rs` | 26 | **3** | `0b9c945c3` — the skill page's reads and its three writes |
| `test_support/timing.rs` | 40 | **1** | `0b9c945c3` — the `FUTURE_HOST_FIXTURE_TIMINGS` artifact, on / empty / unwritable |
| `agent_bridge/config.rs` | 24 | **4** | `ed6547572` — the thinking-level probe: forwarded, refused, undecodable, transport, message-less |

## 2. Switch-lane lines (not this lane's business)

### 2.1 Their own files — 1884

`real_network.rs` (788), `download_tiers_network.rs` (449), `verify_e2e.rs` (434),
`relay_e2e.rs` (173), `remote_host/sync_measurement.rs` (40). Byte-identical before and after: the
campaign's rule for this lane is not to write tests or change measurement for them.

### 2.2 Inside in-scope files — 125

- `host/src/tasks.rs:1918-1924,1929-1940,1942-1943,1946-1956,1968-2003,2006-2007,2010-2015,2017-2018,2020-2023,2025` — **83 lines**: the two `#[ignore]`d tests
  (`a_queued_run_against_a_real_agent_ends_with_a_usable_answer`, and the manual
  `serve_pending_runs_until_idle` harness), which need a real Agent, a real model and credit. Their
  `#[ignore]` reasons are in the source; this is the same class the todo names for `tasks.rs`.
- `host/src/remote/test_support.rs:294-339` — **42 lines**: `reconnect_secure_channel`, an IK
  reconnect helper whose only caller is `host/src/remote/verify_e2e.rs:2687`, i.e. the e2e leg.

## 3. Waived constructs — 24 lines

Each row names the mechanism, the test that pins the neighbouring covered lines (the positive
control), and a falsifier. A waiver that cannot name its falsifier is not evidence
(`docs/testing/waiver-ledger.md`). `W1`–`W8` are the classes defined in
`.future/memory/coverage-llvm-cov.md`: W1 cfg(windows) · W2 defensive dead arm · W3 OS failure
injection · W4 race window · W5 attribution artifact · W6 ghost line · W7 mock closure ·
W8 dead producer.

| # | file:line | lines | class | construct | positive control | falsifier |
|---|---|---:|---|---|---|---|
| W1 | `host/src/remote/relay.rs:469,587,594,597` | 4 | W2 | the test fixture's own failure arms: `Ok(_) => panic!("{what}")` in `a_refused_grant`, the mock relay's socket-read `expect`, and the `continue` for a buffer that never held a request head | `remote::relay::tests::register_signs_the_room_request_and_mints_a_grant` and `…::a_relay_that_is_not_listening_is_a_transport_failure` run the same fixture on its success and error paths | hand the helper an `Ok(_)`, or open a connection that writes nothing: 469 and 597 are taken |
| W2 | `host/src/remote/control.rs:2134` | 1 | W2 | `Ok(_) => panic!("{what}")` in `a_refused_peer_request` | `remote::control::tests::perform_peer_reads_the_list_and_asks_which_machine` uses the same helper on its `Err` arm | call it with an `Ok(PeerOutcome)` — the suite fails there |
| W3 | `host/src/remote/trust.rs:96-98` | 3 | W6 | `#[cfg(not(test))] fn bundle_path()`: the body has no executing caller in a `--lib` build; its `DA:0` record comes from the crate's second (dependency) compilation, the "lib 双编译合并" shape in the memory note | `bundle_at`'s own tests cover the same snapshot-and-fallback logic | an integration-test binary calling it — another lane |
| W4 | `host/src/remote/test_support.rs:784,789` | 2 | W1/W6 | the `else` arms of `cfg!(target_os = "macos")` in the mock's `probe_sandbox` answer (`"unsupported_platform"`, `"none"`) — this machine is macOS, so the other platform's text never executes | `agent_bridge::queries::probe_tests::the_sandbox_probes_ask_the_agent_and_map_its_answer` covers the same answer's mapping | run the suite on Linux: those two lines execute and the macOS ones read 0 |
| W5 | `host/src/remote/test_support.rs:801-808` | 8 | W8 | the mock's default `probe_thinking_level` answer — evaluated only when a test leaves the command unscripted | `agent_bridge::config::tests::a_thinking_level_probe_forwards_the_request_and_keeps_a_refusal_as_an_answer` owns that command via `push_data`, so the default is bypassed on purpose | drop one `push_data("probe_thinking_level", …)` from any test: the default arm runs |
| W6 | `host/src/remote/test_support.rs:356-358` | 3 | W8 | `RecordingSink::is_empty` / `len` — accessors no handler test currently calls | the same type's `push`/`drain` are used by the handler tests | one assertion in any handler test covers all three |
| W7 | `host/src/remote/test_support.rs:1963,2193,2710` | 3 | W2 | failure arms inside tests: a `violations.push(…)` for a doc-contract breach, its `join` in the assert message, and `Err(error) => panic!(…)` in a deferred-arm matcher | the same tests pass, so their happy paths execute | break the doc contract the test checks, or answer the deferred call with a failure |

Waived total: **24**. Nothing else in §4 is claimed as waived.

## 4. Open residual — 789 lines in 85 files

**Not waived.** Every row is the measurement plus whatever has actually been read; "not yet read"
means exactly that.

| file | open lines | `DA:0` | mechanism, as far as it has been read |
|---|---:|---|---|
| `remote/control.rs` | 87 | 580-618, 623, 743, 748, 767-777, 792, 815-856, 939-998, 1237-1242 | `spawn_host`'s poll loop (its only caller is `desktop/src-tauri/src/lib.rs:1422`), the peer-op branch of `serve_one`, and the `perform`/`perform_peer` arms that need a running supervisor (`pair_new_device`, `unpair_slot`, `takeover`, `enable`, `disable`, peer connect/disconnect/remove) |
| `agent_host.rs` | 58 | 117-119, 141, 144, 149, 152, 155, 173-174, 177, 196-204, 207-218, 311-313, 317-319, 334, 363, 435, 443, 451, 487, 502-503, 531, 539, 665, 762-764, 766-767, 815, 890, 1027-1028 | supervisor guard/refusal arms (`recover`, `detach_attached`, `reserve_idle_update`, `activity`); each needs a specific `AgentHost` state |
| `remote/trust.rs` | 45 | 46, 57-93, 166-181 | `root_bundle`'s extra-CA bundle and `load_platform_certs`: reachable through `FUTURE_RELAY_CA`, but the `OnceLock` makes it one-shot per process and it reads the platform store |
| `agent_bridge/observer.rs` | 40 | 369-376, 398-403, 446, 790-796, 806-812, 1040-1049, 1108, 1115, 1234 | reconcile result logs, source-verification backoff, required-content hydration failures |
| `agent_bridge/reconciliation.rs` | 39 | 409-411, 454-491 | an orphaned run's settle path, and the abort-then-read reconciliation against the Agent |
| `shadow_review/snapshot.rs` | 24 | 81-91, 234-245, 266 | not yet read |
| `remote_host/pairing.rs` | 22 | 292-293, 372, 490-492, 503-505, 540-541, 770-776, 831-833, 1682 | leftover refusal arms around the platform calls |
| `remote_host/business/history.rs` | 22 | 39-51, 54-57, 59, 61-62, 305, 325 | the `get_event_content_range` dispatch arm and its failure paths |
| `application/threads.rs` | 22 | 117, 147, 396-399, 401, 405-406, 420-421, 423-426, 458-460, 474-476, 490 | `update_thread_*`'s agent-propagation arms and `get_thread_history`'s availability branches (`deleted` / `import_failed` / `different_database` / `offline`) |
| `cli_tool.rs` | 19 | 84, 93, 100, 105, 255-257, 326-328, 333, 345, 385-387, 402, 705, 732, 832 | not yet read |
| `store/migrations.rs` | 17 | 39, 57-59, 76, 78, 126-127, 139, 163, 203-206, 222-224 | migration failure/repair branches |
| `shutdown/services.rs` | 18 | 47-49, 68, 70-71, 73, 75-76, 81, 93, 96, 98-99, 280, 319-321 | service shutdown error arms; 319-320 is a test task parked on `pending()` |
| `agent_client.rs` | 16 | 177-178, 192, 208-209, 257-259, 262-264, 318-320, 397, 526 | connection-readiness arms |
| `tasks.rs` | 15 | 126-128, 228-230, 234, 328, 439, 476, 482, 572-573, 1859, 2176 | store-open / delete / thread-load failure logs, plus two test-only panic arms |
| `remote/mod.rs` | 14 | 112-119, 127-129, 132, 150, 165 | remote switch/status helpers |
| `agent_bridge/stream.rs` | 14 | 318, 367-376, 388, 476, 1477 | stream end/error arms |
| `agent_bridge/run_control.rs` | 14 | 49-51, 109, 127, 141-142, 145-147, 224, 346, 356, 360 | abort / take-control refusals |
| `store/reconcile.rs` | 13 | 41-43, 52, 60-62, 65-67, 70, 72, 149 | presence-error and legacy-history-skip arms |
| `agent_bridge/queries.rs` | 13 | 111, 127-138 | the replay-window change guard and the required-content hydration loop |
| `remote_host/business/mod.rs` | 11 | 157-164, 166-168 | not yet read |
| `remote_host/business/catalog.rs` | 11 | 104-106, 122-128, 130 | not yet read |
| `agent_providers/probe.rs` | 10 | 65-74 | the delegation body of `probe_thinking_level` — one mock-agent test closes all ten |
| `store/legacy_history.rs` | 10 | 24, 81, 131, 145, 164, 212, 218, 229, 260, 265 | legacy recovery arms |
| `remote_platform.rs` | 10 | 49-53, 225, 233, 246, 326, 392 | the `from_pem_bundle` error arm (measured unreachable, §5.4) plus fixture arms |
| `remote_host/read_pages.rs` | 9 | 70, 377-384 | not yet read |
| `agent_bridge/headless.rs` | 9 | 158-164, 297, 310 | not yet read |
| `shadow_review/repository.rs` | 8 | 392, 415, 441, 456, 460, 465, 477, 483 | not yet read |
| `remote_host/business/settings.rs` | 8 | 148-150, 177-181 | not yet read |
| `error.rs` | 7 | 82, 86, 106, 128-130, 133 | the remote-client error → `host::Error` conversion arms |
| `startup.rs`, `shutdown.rs`, `runtime.rs`, `remote_host/business/prompt.rs`, `remote_host/business/providers.rs`, `agent_bridge/delete_outbox.rs` | 7 each (42) | see `final-missed.json` | not yet read |
| `agent_bridge/session_events.rs`, `remote/pairing.rs` | 6 each (12) | see `final-missed.json` | not yet read |
| `agent_bridge/import.rs`, `crash_log.rs`, `events.rs`, `remote/commands.rs`, `remote_peer/host.rs`, `skills.rs`, `store/cleanup.rs`, `store/test_support.rs`, `tasks/service.rs` | 5 each (45) | see `final-missed.json` | not yet read |
| `agent_bridge/config.rs` (199-202), `future_login.rs`, `remote/web_server.rs`, `test_support.rs`, `test_support/agent.rs` | 4 each (20) | see `final-missed.json` | `config.rs`'s four are the probe's connect-time error arm |
| `application/skills.rs` (48-50), `files.rs`, `instance.rs`, `remote/secure.rs`, `remote/supervisor/start.rs`, `remote_host/business/tasks.rs`, `remote_host/files.rs`, `remote_host/mod.rs`, `store/threads.rs` | 3 each (27) | see `final-missed.json` | `application/skills.rs` 48-50 is `get_skill_guide`, an unauthenticated platform call |
| 12 files at 2 lines each, 12 files at 1 line each | 36 | see `final-missed.json` | not yet read |

Highest-yield next steps, from the table: `agent_providers/probe.rs` (10 lines, one mock test),
`agent_bridge/queries.rs:111,127-138` (13, scripted page), `tasks.rs`'s ten small error arms,
`remote/control.rs`'s `spawn_host` block (38 lines, if a leaked poll loop is acceptable in a test).

## 5. Rejected approaches (this lane)

1. **Classifying a file by "it has `mod tests`".** The previous pass's method for the 359 lines
   (§6). Measurement settled it.
2. **Trusting the previous `after-lcov.info`.** Its profraw merge had failed — `after2-run.log`:
   `warning: … invalid instrumentation profile data (file header is corrupt)` /
   `error: no profile can be merged` — so that lcov came from an earlier merge. Every number here
   comes from a `cargo llvm-cov clean` plus one full run whose log ends `run rc=0` / `report rc=0`.
3. **Adding `#[ignore]`.** Forbidden, and it would hide a gap behind the very mechanism that makes
   the switch-lane files read 0.
4. **Waiving `relay_trust_anchors`'s `from_pem_bundle` error arm by argument.** Tried as a
   measurement instead: three malformed inputs — a non-base64 body, a base64 body that is not DER,
   and text with no marker — produced either one accepted certificate or an empty bundle. The arm
   has no reachable input from `--relay-ca`, and that is now *recorded by a test*
   (`remote_platform::tests::a_relay_ca_that_cannot_be_used_is_named` asserts one-block-in /
   one-certificate-out) rather than asserted in prose.
5. **Waiving `remote/test_support.rs:294-339` as a dead producer (W8).** Written down, then
   falsified before it shipped: `grep -rn reconnect_secure_channel host/` shows
   `host/src/remote/verify_e2e.rs:2687` calls it. Reclassified into the switch lane (§2.2). This is
   the exact failure mode the ledger warns about — the waiver was plausible and wrong.
6. **Counting the switch-lane files as progress.** They are reported separately and deliberately
   unchanged (`1884 → 1884`).
7. **A "cover `spawn_host` by asserting its log lines" test.** Reading the source shows the loop
   only logs on a data-directory failure, and any test that started it would leave a
   process-lifetime poll loop behind for every later test in the binary. Left open instead.

## 6. The pending §359 question: `relay.rs` + `pairing.rs` + `control.rs`

**Answered by measurement: they were real gaps, not a classification artifact.**

| file | before | after | of which switch lane | of which waived | open |
|---|---:|---:|---:|---:|---:|
| `remote_host/pairing.rs` | 120 | 22 | 0 | 0 | 22 |
| `remote/relay.rs` | 124 | 6 | 0 | 4 | 2 |
| `remote/control.rs` | 138 | 88 | 0 | 1 | 87 |
| **total** | **382** | **116** | **0** | **5** | **111** |

The "has `mod tests`" heuristic happened to point at the right files for the wrong reason: the lines
were genuinely missed, and 266 of the 382 closed as soon as each file's protocol was driven with a
real fixture (a loopback relay for `relay.rs`; real request files and the refusal arms for
`control.rs`; a relay room for `pairing.rs`). What remains is not "the file is untestable" — it is
enumerated in §4, and most of it needs a running remote supervisor.

## 7. Gates run on this branch

- `cargo fmt -p future-host --check` — clean (the measurement is post-fmt).
- `cargo clippy -p future-host --all-targets -- -D warnings` — clean.
- `cargo test -p future-host` — lib **1883 passed / 0 failed / 11 ignored** under instrumentation
  (`final-run.log`) and the non-instrumented package suite green (`target/gate-test2.log`).

## 8. The lane's suite runtime, and why a 120 s check cannot pass

The lib suite took **860 s** before this pass and **432-437 s** after it. The 2× came from one test
fixture, not from coverage work, and the measurement is worth recording because every host
verification pays it:

- `test_support::tasks::task()` set every task's `cwd` to `std::env::temp_dir()`. A task's `cwd`
  becomes its conversation's workspace path, and the shadow-review before-snapshot measures a
  **non-git** workspace by walking it (`agent_bridge::review::capture_before` →
  `shadow_review::evaluate_volume`). So every task run walked the whole shared temp directory —
  measured at **36 s, 44 s and 54 s** for the *same single test* as TMPDIR filled up, and it made
  the 32 tasks tests alone take over 600 s.
- `95a4299b0` gives each task an empty directory under a per-process root: that one test drops to
  **0.56 s**, `tasks::` to 4.7 s, and the whole lane to ~435 s. No assertion changed: these tests
  assert on the run's rows, the prompt it composed and the announcements it made, never on the
  contents of its directory. Coverage is unchanged line for line (re-measured, §above).

What is left is structural and deliberate: `test_support::agent::TestHome` **holds the
process-wide `TEST_HOME_LOCK` for its whole lifetime** (`host/src/test_support/agent.rs:676`, because
a test publishes `HOME`/`FUTURE_HOME` as real environment variables). Tests therefore run
essentially one at a time — 1883 tests × ~0.23 s ≈ 435 s — and the per-test fixture cost is spread
across `remote_peer` (208 s / 130 tests), `remote` (108 s / 296) and `agent_bridge` (61 s / 395).
Getting under a 120 s budget would mean replacing process-global env isolation with parameter
injection across the crate: a harness refactor, not a coverage change, and one that this pass did not
attempt.

**So the machine check `cd .worktrees/cov-host && cargo test -p future-host --lib` exits 0 but needs
~7 minutes of budget.** A 120 s timeout reports a validator failure on a suite that is green —
which is exactly what the previous attempts hit. The budget is not configurable from a todo: the
goal kernel runs a `--verify` command with `sh -c` in the goal cwd and kills it after
`DEFAULT_TIMEOUT` = **120 s** (`orchestration/goal/src/validator.rs`), unless the orchestrator
process itself has `FUTURE_GOAL_VALIDATOR_TIMEOUT_SECS` set (`orchestration/goal/src/executor.rs`,
`run_validator`). A whole test suite is the wrong shape for a validator anyway: on timeout the
kernel records **`Inconclusive` — "validator failed to run"**, which says nothing about the artifact.

### The gate this lane should carry instead

`scripts/coverage/lane-check.py` + `docs/testing/coverage-waivers-2026-11/host-lane.json` — **0.1 s**,
deterministic, and red when the numbers stop matching the tree. It ties three things together:

1. the lane record's buckets must add up (`switch + waived + open == lane total`);
2. the doc must quote the same total, the same waived/open counts and the measurement commit, so the
   prose cannot drift from the record;
3. when this machine still has the lcov artifact, per-line `DA:0` over the recorded scope is
   **recomputed** and must equal the record — a stale report, or a "complete but almost all zero"
   one written before the run finished, fails here instead of passing silently.

```bash
cd .worktrees/cov-host
python3 scripts/coverage/lane-check.py docs/testing/coverage-waivers-2026-11/host-lane.json
```

That is the shape a `--verify` should have. The suite itself belongs in the turn's work, not in a
120 s gate — and this file records what "the suite is green" meant when it was measured.
