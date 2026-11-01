# host (Rust, `cargo test -p future-host --lib`) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The llvm-cov
summary's `Lines` column is not the acceptance number (it adds ghost lines).

**Lane** — `future-host`'s **lib** unit tests only. The integration-test binaries and the
`#[ignore]`d real-network legs are measured by their own lanes; this file is about the lib.

**Last measurement**: `test/cov-host` at `0bf7f96d9`, rustc **1.99.1** (`rust-toolchain.toml`),
`HOME=target/test-home`, `ulimit -n 8192`. Artifacts (gitignored) are in
`coverage/2026-11/after/host/`: `round11-C-lcov.info` (pass C — the tree the branch is at, and the
numbers quoted below), `round11-C-run.log`, the per-line inventory `round11-C-missed.json`, the
open-line dump `round11-C-open.json` and the bucket arithmetic in `buckets11c.py`. The pair that
measures the tree before §11.7's seam fix is `round11-A-*` / `round11-B-*` (an independent repeat,
§1.1) with `buckets11.py` and `round11-open.json`; `round11-falsify.log`
(`round11-falsify-run.log` for its restore) is the round's mutation log. `round11-base-*` is the
same-recipe probe this round started from (`d3b2c58fd`, **2394**). Superseded by this round:
`round9-A/B-*` and `round9-tip-A-*` (2392 / 2394), `round7-A/B-*` (2464), `round6-A/B-*` (2505),
`round5-A/B2-*` (2553), `base3-*` (2589) and `final-*` (2589).

`compare11c.py` is the pass-A-to-pass-C check: it recomputes both line sets from the two lcov files
and asserts that C's `agent_host.rs` rows are A's under the two whole-line shifts §11.7's edit makes,
and that no other file moved except on the three lines §1.2 already names as movers.

The lane record beside this file — `host-lane.json` — carries the same numbers, and
`scripts/coverage/lane-check.py` fails when the record, this doc and the lcov artifact disagree
(§8). Run it after every re-measurement; it is the cheap gate this lane can actually carry.

**The numbers in §1–§4 are pass C's** (`0bf7f96d9`, **2378**), measured on the tree that carries
§11.7's seam fix. The pair measured on the tree before that fix is `c6b7b4979`'s: **2375** (A) and
**2377** (B), two lines apart, both named (§1.1). Round 9's record was `b50efdb55` (2392) and the
tip it landed on read 2394; round 10 added no fixture to this lane, so 2394 is the number this
round started from (`round11-base-*`).

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

The run is also the gate: **1979 passed, 0 failed, 11 ignored** in 438.66 s (A), 435.07 s (B) and
432.61 s (C), each with `report rc=0` (`round11-A-run.log`, `round11-B-run.log`,
`round11-C-run.log`). Both A and B were measured at `c6b7b4979`; C is the same recipe after
§11.7 and is the artifact of record.

## 1. Before / after, and where every remaining line sits

Both columns are `DA:0` over `host/src/`, same lane, same recipe. "before" is `round11-base-lcov.info`
— a probe at this round's starting tree (`d3b2c58fd`), which is round 9's 2392 plus the two lines
§10.6 named; "after" is pass C at `0bf7f96d9`; the pass-A column is the same tree as C minus
§11.7's seam, so the two can be read against each other; and the round-9 record
(`round9-A-lcov.info`, `b50efdb55`) is quoted last.

| | before (`round11-base`, `d3b2c58fd`) | after (pass C, `0bf7f96d9`) | Δ | pass A (`round11-A`, `c6b7b4979`) | round 9 (`round9-A`, `b50efdb55`) |
|---|---:|---:|---:|---:|---:|
| **lane total** (`host/src/**`) | 2394 | **2378** | **−16** | 2375 | 2392 |
| switch-lane files (§2.1) | 1884 | 1884 | 0 | 1884 | 1884 |
| **inside in-scope files, switch lane only** (§2.2, §2.3) | 132 | 132 | 0 | 132 | 132 |
| **in-scope** (lane total − §2.1 files − §2.2/§2.3 lines) | 378 | **362** | **−16** | 359 | 376 |

Every one of the 2378 remaining lines is in exactly one bucket (`buckets11c.py` prints this; the
three numbers sum to the total):

| bucket | lines | what it is |
|---|---:|---|
| **switch lane** | **2016** | §2.1 — 1884 in five files only the network legs execute — plus §2.2/§2.3 above |
| **waived** | **95** | §3 rows W1-W20: 76 lines in named rows plus 19 closing edges |
| **open** | **267** | §4 — real gaps. **Not waived.** |

So the campaign figure for this lane today is **267 open**, not 0. This round's tests closed **17
in-scope lines by execution** — six in `agent_host.rs` (`144`, `173-174`, `216-217`, `334`) and
eleven in `remote/trust.rs` (the `bundle_with_extra` block and the newline guard, §11.2/§11.3) — and
the base probe's 2394 → pass C's 2378 is **−16**: those 17, less the one line that is covered in the
base probe and in pass A but `DA:0` in C, the race-attributed `agent_host.rs:565` (old `502`,
§1.2). `buckets11c.py` departs from `buckets11.py` in exactly one place: `buckets11.py`'s
closing-edge rule would absorb `agent_bridge/presence.rs:112` into W8 (that rule's answer for this
pass is 266 open / 96 waived), while §1.2's rule keeps a line that flips between passes in §4 — so
W8 stays at its 19 lines and that line is one of the 267.

`buckets11c.py` asserts, before it counts, that every §3 waiver line and every §2.3 fixture arm is
still `DA:0` — a waiver this measurement would have falsified, or an arm it would have covered,
fails the run instead of being silently retired — and that every closing-edge line it does accept
has a covered neighbour. The same assertions fired once during this round, on
`remote/test_support.rs`, because the tree moved under W7's three lines (§10.6 fixed the same class
of shift by hand); they are re-pointed at `2078/2308/2875` and every pass since has held. W3 (§3) is
retired outright: its three lines read **>0** in every pass this round took, so the waiver no
longer describes the artifact it claims (§11.6).

Where this measurement's closed lines came from (per file, before → after, raw `DA:0` including the
waived lines of the same file):

| file | before | after | what closed it |
|---|---:|---:|---|
| `agent_host.rs` | 25 | **20** | round 11 (§11.2): the recovery refusal, the incomplete-cleanup arm, the cancelled-start answer and the owned idle reservation, each driven in a process of its own — six lines closed, and §11.4 records that `144` is executed but not mutation-pinned. The nine that moved line numbers (`762-767, 815, 890, 1027-1028` → `858-860, 862-863, 911, 1046, 1183-1184`) are the same fixture failure arms, shifted by this round's insertions and again by §11.7's seam; `565` (the race-attributed old `502`) reads 0 in this pass, which is why this row is 20 and not 19 |
| `remote/trust.rs` | 17 | **6** | round 11 (§11.3): the three refusal arms of `bundle_with_extra` and the trailing-newline guard, one process each. The six left are `load_platform_certs`'s error collection (`169-174`), which only a failed platform read reaches |
| `shutdown/services.rs` | 17 | 17 | unchanged in this column: `20` reads `DA:0` in the base probe as well, and it takes different values across passes (§1.2) |
| `agent_bridge/presence.rs` | 2 | 2 | unchanged in this column for the same reason — `112` is a mover, and pass C reads it 0 like the base probe did (§1.2) |

Every line this column moves is named: **17** were closed by this round's new tests, and **1** of
§1.2's three movers (`agent_host.rs:565`) is `DA:0` in C where the base probe read it covered —
17 − 1 = **16**. The other two movers read 0 on both sides of this column, so they change nothing
here and are carried in §4.

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

Pass B (`round11-B.sh`) runs the identical recipe on the same instrumented build with its
`.profraw` files deleted first, into an independent lcov file. Both passes are green and complete:
**1979 passed / 0 failed / 11 ignored** in 438.66 s (A) and 435.07 s (B).

| | pass A (`round11-A-lcov.info`) | pass B (`round11-B-lcov.info`) |
|---|---:|---:|
| lane total `DA:0` | **2375** | **2377** |
| switch-lane files | 1884 | 1884 |
| in-scope | 359 | 361 |
| lines that differ between the runs | — | **2**, both named in §1.2 |

A and B disagree on exactly two lines, both of which had already been named as movers before this
round (`agent_host.rs:502`, now `565`) or moved because of what an earlier test in the binary leaves
behind (`agent_bridge/presence.rs:112`), so the pair is still a reproduction of the same tree — a
2-line band, not a drifting number. `perfile11.py round11-A-missed.json round11-B-missed.json`
prints it.

#### 1.1.1 Pass C — the tree after §11.7's seam, and why it is the record

`round11-C-lcov.info` repeats the identical recipe on `0bf7f96d9`, the commit that replaces the
`set_var` start-mode test with a thread-local seam (§11.7). It is the artifact of record because it
is the tree the branch is at: **1979 passed / 0 failed / 11 ignored in 432.61 s**, `run_rc=0`,
`report_rc=0`.

| | pass A (`c6b7b4979`) | pass C (`0bf7f96d9`) | Δ |
|---|---:|---:|---:|
| lane total `DA:0` | 2375 | **2378** | **+3** |
| files the difference touches | — | **3** | |

Those three lines are exactly §1.2's three movers, each on its `DA:0` side in C:
`agent_bridge/presence.rs:112`, `shutdown/services.rs:20` and `agent_host.rs:565`. Nothing was
closed and nothing new was opened, and `compare11c.py` proves it rather than asserting it: it
recomputes both line sets from the two lcov files and checks that A's 19 `agent_host.rs` rows map
onto C's 20 by the two whole-line shifts the edit makes — **+66** for old lines above `365` and
**+63** below, because the new seam adds 66 lines at old `111` and the four-line mode check at old
`361-364` becomes one line — and that no row appears or vanishes in any other file.

What the pass had to prove is the other half — that the new lines are covered, so the seam does not
buy a test-fix with uncovered product lines. Every line of `start_mode`, `launch_mode`, the
thread-local slot and `StartModeGuard`'s `set`/`drop` reads `DA>0` in C: the two `select` tests
between them take `launch_mode`'s armed arm (`a_start_mode_the_entry_does_not_understand…`), its
fall-through arm and `start_mode`'s `Ok` path (`a_local_start_permission_is_honoured_only_…`), and
`start_mode`'s `Err` arm. The `DA:0` count moved by the three movers alone.

**Three passes were discarded rather than read this round**, and none of them is a number:

- `round11-A-flake-run.log` — a complete run with **one** failing test
  (`remote::control::tests::serving::a_client_role_change_runs_against_the_paired_machine`, which
  answered `phase: Stopped` from its bridge start and panicked in the fixture's own guard,
  `remote_peer/testing.rs:70`). The same tree then ran the whole `remote::` module three times
  (317 tests, 3/3 green) and the whole suite twice more (A and B, both green), so the failure did
  not reproduce; it is recorded here as a load-sensitive fixture failure and **not** as a verdict
  on this round's work. Its lcov was overwritten by the valid A pass, which is deliberate: a red
  pass has no verdict and must not leave a report that could be quoted.
- `round11-B` (first attempt) — killed while I still had test edits to make, because a pass measured
  on a tree that then changes is not a repeat of anything.
- `round11-focused-*` — a filtered run (`-- remote::trust`, `-- agent_host::block_fixtures`) used
  only to confirm the new fixtures hit their lines before paying for a whole-lib pass.

Earlier rounds' discards — `round9`'s three, `round8-full-lib-run.log` (parked), `round7-A-DISCARDED-*`,
`round6-B-DISCARDED-*`, `round5-B-DISCARDED-sigkill`, `round4-A-DISCARDED-two-failures` — are the
same rule: **a pass that did not finish green has no verdict, and its partial `.profraw` is not a
measurement.**

### 1.2 Lines that move between passes, named

| line | what it is |
|---|---|
| `agent_host.rs:565` (old `502`) | `server.as_mut().await?` in the shutdown-report arm of the start loop. `DA:0` in B and in C, `>0` in A and in the base probe: whether the serve future is awaited after the shutdown report depends on which of two ready futures the select sees first. The number moved by §11.7's seam (`+63`). §4 keeps it as an open row and this table keeps it named |
| `agent_bridge/presence.rs:112` | the `}` closing the attach-presence block. `DA:0` in the base probe, `>0` in A, `DA:0` in B and in C: it reads whatever the last test to publish a presence event left behind. It was W8's closing-edge row in round 9 and is **not** claimed as covered here — `buckets11c.py` deliberately does not offer it the closing-edge waiver (§1) |
| `shutdown/services.rs:20` | `return Err(error.into())` when `super::failure()` already holds a recorded failure. `DA:0` in the base probe and in C, `>0` in both A and B: an earlier test in the binary decides whether the process-global failure is still set. It reads 0 in some runs, so §4 keeps it |

The rule this table carries: a line that takes different values in two passes of the same tree is
**not** a closure and **not** a waiver. It is a line whose input is another test's leftovers, and it
stays in §4 with the reason written down.

`remote_host/pairing.rs:1869` is the fourth of this family, and it is stable now for a reason worth
keeping: the `RelayEnv` fixture's drop arm executes only when `FUTURE_RELAY_URL` was already set
before the guard was created, which is decided by test order inside the file (round 7's order did,
round 9's and this round's do not).

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

## 3. Waived constructs — 95 lines

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
| W3 | `host/src/remote/trust.rs:96-98` | **0** | **retired** | `#[cfg(not(test))] fn bundle_path()` was waived as a ghost line of the crate's second (dependency) compilation, with no executing caller in a `--lib` build | — | **it does not reproduce**: every pass this round took reads `DA:96,>0`, `DA:97,>0`, `DA:98,>0` (base `1`, A `4`, B `4`), so the lines are covered and the old row described a report that no longer exists. The line count is zero rather than three, and §11.6 records the correction |
| W4 | `host/src/remote/test_support.rs:784,789` | 2 | W1/W6 | the `else` arms of `cfg!(target_os = "macos")` in the mock's `probe_sandbox` answer (`"unsupported_platform"`, `"none"`) — this machine is macOS, so the other platform's text never executes | `agent_bridge::queries::probe_tests::the_sandbox_probes_ask_the_agent_and_map_its_answer` covers the same answer's mapping | run the suite on Linux: those two lines execute and the macOS ones read 0 |
| W5 | `host/src/remote/test_support.rs:801-808` | 8 | W8 | the mock's default `probe_thinking_level` answer — evaluated only when a test leaves the command unscripted | `agent_bridge::config::tests::a_thinking_level_probe_forwards_the_request_and_keeps_a_refusal_as_an_answer` owns that command via a script, so the default is bypassed on purpose | drop one `probe_thinking_level` script from any test: the default arm runs |
| W6 | `host/src/remote/test_support.rs:356-358` | 3 | W8 | `RecordingSink::is_empty` / `len` — accessors no handler test currently calls | the same type's `push`/`drain` are used by the handler tests | one assertion in any handler test covers all three |
| W7 | `host/src/remote/test_support.rs:2078,2308,2875` | 3 | W2 | failure arms inside tests: a `violations.push(…)` for a doc-contract breach, its `join` in the assert message, and `Err(error) => panic!(…)` in a deferred-arm matcher | the same tests pass, so their happy paths execute | break the doc contract the test checks, or answer the deferred call with a failure |
| W8 | 19 lines, listed in `coverage/2026-11/after/host/closing-edges-round7.txt` (`store/legacy_history.rs:24,81,212,260,265`, `store/migrations.rs:39,76,139`, `agent_bridge/stream.rs:318,388`, `agent_bridge/import.rs:632`, `application/threads.rs:420`, `store/cleanup.rs:456`, `store/deletions.rs:26`, `store/managed_cleanup.rs:28,38`, `store/review_snapshots.rs:203`, `store/runs.rs:184`, `store/approval_assessments.rs:63`) | 19 | W6 | a line that holds **nothing but** a call's closing error edge (`)?;`, `})?;`, `)?`) — "多行调用把 `)?;` 单独落一行 → 该行只有错误臂 → `DA:0`" (`.future/memory/coverage-llvm-cov.md`) | the statement itself ran: the line directly above each of these 19 reads `DA>0` (checked mechanically by `buckets11.py`). The closing-edge lines whose neighbour is *also* uncovered stay in §4 (`agent_bridge/config.rs:202`, `agent_client.rs:178,209`, `remote/web_server.rs:61`, `remote_host/pairing.rs:293`, `remote_platform.rs:53`, `remote/test_support.rs:67`, `agent_bridge/observer.rs:1049`) | put one of these calls on a single line (shrink its argument list into rustfmt's 60-column `fn_call_width`) and the line disappears from the next report with no test change; conversely a caller that actually forces the error edge — a closed store, an unreadable file — covers it |
| W9 | `host/src/remote_host/business/catalog.rs:104-106,122-130` | 11 | W2 | the two "the fork produced nothing usable" arms of the phone's `fork_session`, plus the `get_thread` failure beside them. All three sit behind one invariant: `agent_bridge::fork_agent_session` refuses a fork whose session id came back empty (`agent_bridge/session.rs:316-318`) and *creates* the thread itself, with that session id bound, before returning its id (`session.rs:373-384`, `get_or_create_thread_for_agent_session(… agent_session_id: Some(new_session_id))`) | `business::catalog::dispatch_tests::fork_session_creates_and_reports_the_child_conversation` and `…::a_refused_fork_is_reported_with_the_agents_reason` drive the same call on the success path and the refusal path, and both read the child back through the same `get_thread` | make `fork_agent_session` return an id it did not persist (a session the Agent forked on another machine, or a fork that returns the parent's id) — then 122-128 answers and all eleven lines execute |
| W10 | `host/src/remote_host/business/settings.rs:148-150,177-181` | 8 | W1 | the sandbox probe's non-answer arms: `product_sandbox_status()`'s `Err` (148-150, 179-181) and `product_sandbox_available()`'s `Ok(false)` (177-178). On macOS that function is `#[cfg(target_os = "macos")] { Ok((true, None)) }` (`settings.rs:301-310`): the sandbox is a constant there, so neither the `Err` nor a `false` availability has an input in this lane | `business::settings` tests drive `get_settings` and the settings write through `product_sandbox_available()`, so the macOS arm itself is covered | run the lane on Linux or Windows, where the answer comes from `agent_bridge::probe_sandbox()`: a probe failure reaches 148-150 and 179-181, and a host without a sandbox reaches 177-178 |
| W11 | `host/src/shutdown.rs:186-187,204-208` | 7 | W3 | the two `std::thread::Builder::new().spawn(…)` failures of the stop path: the cleanup watchdog (186-187) and the cleanup worker (204-208). Thread creation fails only when the OS refuses it | `shutdown`'s tests run the same `stop()` to completion, including the worker's finish/progress reporting that the arms surround | exhaust the process's thread budget (lower `RLIMIT_NPROC`/`ulimit -u` to what is already in use) and call `stop()`: `pthread_create` fails, `spawn` returns `Err`, and both arms answer |
| W12 | `host/src/remote_host/business/providers.rs:58-64` | 7 | W2 | `serde_json::to_value(view)` in `reply_view`, where `view: agent_providers::ProvidersView` is `Vec<BuiltinProvider>` + `Vec<CustomProvider>` of `String`/`bool`/`usize` fields (`agent_providers/mod.rs:59-76`). `to_value` fails only on a non-string map key or a non-finite float, and neither type occurs in the view | every provider test answers through `reply_view`, so the same `to_value` succeeds on the same types | add a field whose `Serialize` can fail (an `f64` that can be NaN, or a map keyed by a struct) — the arm then has an input and answers |
| W13 | `host/src/shadow_review/snapshot.rs:266` | 1 | W6 | the `paths = paths.len(),` field value of `try_stage`'s "skipped paths that no longer exist" warning. `tracing`'s field expressions are evaluated only inside the callsite's enabled block, and this one has **no hit counter of its own**: the region belongs to the callsite construction on `264` | `shadow_review::snapshot::tests::stage_warns_with_the_candidate_count_when_a_pathspec_matches_nothing` installs a thread-scoped subscriber and **asserts the recorded value** (`paths = 3`) — so the expression runs, on a line the report calls 0. The same walk through `enabled`/`new_span`/`record`/`record_follows_from`/`enter`/`exit` leaves the fixture itself with no unexercised method | `cargo llvm-cov -p future-host --lib` with that test present still prints `266` as `DA:0` while the assertion passes (measured again this round, `round7-A-missed.json`); delete the assertion and the run fails at `Some(3) != None` — the line is *executed*, so this is attribution, not reachability |
| W14 | `host/src/remote/control.rs:584-586` | 3 | W3 | `directory()`'s failure arm: `Err(error) => { eprintln!(…); return }`. It answers only when the FutureOS data root cannot be resolved at all — no `HOME`, no `USERPROFILE`, no passwd home — which no fixture can arrange without breaking every other test in the same binary | `…::the_control_loop_creates_its_directory_and_answers_what_it_finds` and `…::the_control_loop_gives_up_on_a_directory_it_cannot_create` run the same `directory()` call on its `Ok` path, and the second drives a **refused** `create_dir_all` into its own report-and-return | start the loop in a process whose home cannot be resolved (`env -u HOME -u USERPROFILE` with no passwd entry): `future_dir()` fails and 584-586 answer |
| W15 | `host/src/remote/control.rs:839` | 1 | W2 | the takeover match's catch-all `Err(error) => Err(error)`. `Supervisor::takeover` has exactly two outcomes — `device_offline` when this process holds no runtime for the device, and the status it just took | `…::a_takeover_hands_the_machine_over_or_names_a_device_with_no_bridge` drives both of those, and asserts the `device_offline` arm's own sentence | add a third failure to `Supervisor::takeover` (a status that cannot be projected, a `publish_control` that fails): the catch-all then has an input |
| W16 | `host/src/remote/control.rs:989-993` | 5 | W2 | the client role's unknown-op refusal, behind the router's own decision: `serve_one` sends a request to `perform_peer` only when `is_peer_op` accepts it, and `perform_peer` answers all five operations that function accepts | `remote::control::tests::the_client_role_operations_are_told_apart_from_the_host_ones` pins `is_peer_op`'s membership — the two sets agreeing is what makes this arm unreachable | add an op to `is_peer_op` without an arm for it (say `OP_ENABLE`): the router forwards it and 989-993 answer instead of the request reaching `perform` |
| W17 | `host/src/remote/control.rs:1238-1243` | 6 | W8 | the operation future inside `an_unknown_operation_is_refused_without_performing_anything`: the test hands `answer` a future that would set `performed = true` and return a status, and its whole point is that the future is **never polled** | the neighbouring `answer` tests hand the same shape to the same call and it does execute — `a_status_without_an_invitation_is_an_error_reply` runs `async { Ok(refused) }` to its own assertion | poll it: give that test a *known* op (`OP_STATUS`) and all six lines execute — which is exactly what its `assert!(!performed)` exists to prevent |
| W18 | `host/src/agent_bridge/observer.rs:1049` | 1 | W8 | the closing brace of the hydration refusal's `if let Err` block, whose only statement is `return false` — no input can execute the brace | `…::run_observer_defers_an_attach_whose_source_it_cannot_verify` drives the same `if let Err` on its refusal path, and its neighbours 1047-1048 are covered in this measurement (`round9-A-missed.json` lists only 1049 for the file) | make the block's last statement fall through instead of `return`: the brace then executes with no test change |
| W19 | `host/src/remote_host/pairing.rs:372` | 1 | W8 | the closing error edge of `PairingIdentity::new(code_expires_at)?` — `future_remote_crypto::generate_identity()` is the call inside it, and it fails only when the OS RNG fails | `…::create_pairing_success` runs the same `PairingIdentity::new` on its success path and `…::a_pairing_code_without_an_expiry_is_refused_by_name` covers the `ok_or_else` one line above, so 369-371 read `DA>0` in this measurement while 372 reads 0 | make `generate_identity` fail (a wrapper that returns `Err` once): 372 executes and the invitation is refused |
| W20 | `host/src/remote_host/pairing.rs:1352,1404,1451,1477` | 4 | W2/W5 | round 9's own test-side lines: the catch-all `other => panic!("expected the platform refusal…")` of two `match error` blocks (a passing test never takes it), and two hoisted `Path::display()` message arguments (`legacy.display()`, `pending.display()`) that an `assert!` evaluates only on failure | each of the two tests fails at the arm assertion the moment the platform's refusal stops arriving — measured in `round9-falsify.log` (T1, T2 for the catch-alls; T3, T5 for the assertions next to the hoisted arguments) | make the platform answer a non-`Remote` error in the first two tests, or make the unlink succeed in `removing_the_last_slot…`: the catch-all and the message argument both execute |

Waived total: **95** (18 in rows W1-W7, 19 in W8, 33 in W9-W12, 1 in W13, 15 in W14-W17, 1 in W18,
1 in W19, 4 in W20, and **0** in the retired W3). Nothing else in §4 is claimed as waived. W14-W17 were round 7's: the lines
that had been counted as *open* and are executed-but-unreachable from this lane for the reasons in
their rows. W18 and W19 are the two round 8 flagged for the next full pass — `observer.rs:1049` and
`pairing.rs:372` — and this measurement is that pass: `waived` 90 → 96, `open` 358 → 280. W20 is
round 9's own: four test-side lines its new tests add, named here rather than netted into the file
count.

Two corrections this round, both from reading the rows against a measurement instead of against each
other: **W3 is retired** — its three ghost lines read `>0` in every pass this round took (base `1`,
A `4`, B `4`), so the row described a report that no longer exists (§11.6) — and **W8 loses one of
its twenty**: `agent_bridge/presence.rs:112` reads `>0` in pass A and `DA:0` in pass B, which makes
it a line decided by what an earlier test left behind, so §1.2 keeps it named instead of §3 keeping
it waived. The line sums still account for the total: 4+1+0+2+8+3+3+19+11+8+7+7+1+3+1+5+6+1+1+4 = **95**.

## 4. Open residual — 267 lines in 63 files

**Not waived.** Every row is the measurement plus whatever has actually been read; "not yet
read" means exactly that. Rows are `DA:0` line numbers in the tree this file records
(`0bf7f96d9`, pass C); `buckets11c.py` writes them as `round11-C-open.json`, and `sec4-drift.py`
re-parses this table against that file so a typed row cannot drift from the artifact.

Round 11 changed five rows here. Two were closed by its own tests and are re-pointed from 17-26
lines down to 6 and 20 (`remote/trust.rs`, `agent_host.rs`); `shutdown/services.rs` and
`agent_bridge/presence.rs` gained back the two lines §1.2 names as movers, which read `DA:0` in
this pass; and `remote/test_support.rs` appears for the first time with the two lines §10.6 had
left unassigned (`1119`, `2602`) — §10.6 said they could not be bucketed without a re-measured
pair, and this pass is that pair. `cli_tool.rs` (22) is still another lane's change in the same
tree, and `remote_peer/testing.rs` (1) is still round 8's regression guard.

| file | open lines | `DA:0` | mechanism |
|---|---:|---|---|
| `agent_host.rs` | 20 | 243,377-379,514,550,565-566,594,602,728,858-860,862-863,911,1046,1183-1184 | the closure's own `is_stopping()` read (`243` — `177` in the old numbering; it is unreachable while the first guard above it refuses, and §11.4's M2 is what shows it: remove that first guard and this one answers instead, with the same sentence), `Stopped` with no yield reason (`514`, old `451` — the attach-only refusal's last arm; `363`, `435` and `443` were closed in round 9, §10.3), worker-spawn failure (`377-379`, old `311-313`), the stop/serve precedence arms (`550`, `566`, `594`, `602`, `728` — old `487`, `503`, `531`, `539`, `665`), the fixture harness's own failure arms (`858-860`, `862-863`, `911`, `1046`, `1183-1184` — old `795-800`, `848`, `983`, `1120-1121`), and the race-attributed `565` (old `502`, §1.2). The numbers moved twice: round 11's fixtures added 66 lines at old `111`, and §11.7's seam added 63 more below old `365` |
| `cli_tool.rs` | 22 | 84,93,100,105,255-257,268-270,339-341,346,358,413-415,430,733,760,860 | **not this lane's work.** The cli lane's wave 16 (`d3fde9dcd`, `b5f1e7c89`, `9a1a89a87`) landed in `dev-huichen` between this lane's two measurements: it closed twelve of round 7's rows here (326-328, 333, 345, 385-387, 402, 705, 732, 832) and added fifteen new ones — the probe deadline's per-thread override (268-270), the discovery child's kill-and-report arms (339-341, 346), the two output-limit refusals (358), the discovery failure report (413-415), the pre-start report (430) and four message arguments (733, 760, 860). Measured here, read by that lane |
| `remote/trust.rs` | 6 | 169-174 | only `load_platform_certs`'s error collection is left: the `bundle_with_extra` arms behind its `OnceLock` (old 64-93) and the trailing-newline guard (old 87) were all closed by round 11's one-process-per-case fixtures (§11.3), which is why this row fell from 17 |
| `shutdown/services.rs` | 17 | 20,47-49,68,70-71,73,75-76,81,93,96,98-99,280,321 | service shutdown error arms and two closing braces, plus `20` — `return Err(error.into())` when `super::failure()` already holds a recorded failure, which reads whatever an earlier test in the binary left set (§1.2: `DA:0` in the base probe and in C, `>0` in A and B) |
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
| `remote/test_support.rs` | 2 | 1119,2602 | the two lines §10.6 named and could not bucket without a re-measured pair: `1119` is the eviction arm of the mock's `PublishLog::record` (`RECORDED_PUBLISHES` is 2048, the heaviest fixture publishes 300 — reachable, not reached), and `2602` is the `matches!(` opening line of a test's own assertion, a macro-attribution line whose neighbours are covered (§3's W13 is the same family) |
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
| `remote_peer/testing.rs` | 1 | 70 | the fixture's own `panic!`: "a host bridge that started must show an invitation". The `let … else` arm runs only when a started bridge answers no invitation — the exact failure round 8's `cleanup_host` guard exists to catch (§9.2), which is why a green suite never takes it |
| `remote_peer/runtime.rs` | 1 | 355 | not yet read |
| `remote_peer/host.rs` | 1 | 158 | the assert-message argument of the `convert` test |
| `remote_host/session_files.rs` | 1 | 74 | a `metadata()` failure between listing and stat |
| `remote_host/pairing.rs` | 1 | 1869 | the `RelayEnv` fixture's restore arm — it executes only when `FUTURE_RELAY_URL` was already set, so its coverage is decided by test order (§1.2). The file's 21 other lines were closed in round 9 (§10.2); of the two left, one is W19 and four are W20 |
| `remote_host/business/transfers.rs` | 1 | 59 | not yet read |
| `error.rs` | 1 | 106 | a remote-client error → `host::Error` conversion arm |
| `build_info.rs` | 1 | 171 | not yet read |
| `auth_store.rs` | 1 | 281 | not yet read |
| `agent_bridge/presence.rs` | 2 | 86,112 | `86` is not yet read; `112` is the `}` closing the attach-presence block, which reads whatever the last test to publish a presence event left behind and therefore moves between passes (§1.2) |

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

**Round 9 note.** The paragraph above is round 7's account of why `perform_peer`'s changing half was
still open. Round 8 built the fixture and drove it (§9.2: the `suspended` host role outliving its
fixture was the blocker, not the pairing), and round 9's measurement is the first that sees the
result: `remote/control.rs` now has **16 open lines, all of them W14-W17**, and no part of the
supervisor or client-role block is in §4.

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

| file | before | after (`round9-A-lcov.info`) | of which switch lane | of which waived | open |
|---|---:|---:|---:|---:|---:|
| `remote_host/pairing.rs` | 120 | 6 | 0 | 5 (W19, W20) | 1 |
| `remote/relay.rs` | 124 | 4 | 0 | 4 | 0 |
| `remote/control.rs` | 138 | 16 | 0 | 16 (W2, W14-W17) | 0 |
| **total** | **382** | **26** | **0** | **25** | **1** |

The "has `mod tests`" heuristic happened to point at the right files for the wrong reason: the
lines were genuinely missed, and 270 of the 382 closed as soon as each file's protocol was driven
with a real fixture (round 5), then another 41 in round 7 (§1), and the rest as those files'
fixtures matured: round 8 measured `remote/control.rs`'s client role (§9.2) and round 9 measured
`remote_host/pairing.rs`'s sweep and unlink refusals (§10.2). Of the 26 lines left, 25 are §3 rows
and **one** is open — the `RelayEnv` fixture arm whose coverage is decided by test order (§1.2).

## 7. Gates run on this branch

- `cargo fmt -p future-host --check` — clean (the measurement is post-fmt).
- `cargo clippy -p future-host --all-targets -- -D warnings` — clean. The one fixup this round was
  `cargo fmt` reflowing the new fixtures' call sites; nothing after the measurement changed a line
  the report records.
- `cargo test -p future-host` — lib **1969 passed / 0 failed / 11 ignored** in 431.55 s, twice
  (§1.1: the repeat is 520.38 s and agrees line for line). Round 8's parked and overflowed
  passes are accounted for in §1.1 and §10.1.
- `python3 scripts/coverage/lane-check.py docs/testing/coverage-waivers-2026-11/host-lane.json` —
  `ok: host/rust-lib at b50efdb55 — 2392 uncovered (2016 switch lane / 96 waived / 280 open)`,
  with the artifact recomputed and the per-line inventory compared (`lcov agrees`, `inventory
  agrees`).
- Round 8's 7 tests are: 4 in `remote::control::tests::serving` (invitation from a Desktop that is
  on, a removal through the supervisor, a takeover answered and one refused, the terminal switch
  stored both ways) and 3 loop/fixture tests (`spawn_host` answering and sweeping, `spawn_host`
  giving up on a `run` path it cannot create, `requests()` on an unreadable directory). Round 9's 6
  are listed in §10.2 and §10.3.

## 8. The lane's suite runtime, and why a 120 s check cannot pass

The lib suite takes **431-520 s** (round 9's two measured passes; it was 426-435 s in round 7 on a smaller suite). The 2× that used to sit on top of that
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
essentially one at a time — 1969 tests × ~0.22 s ≈ 430 s.

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
`lane-check.py` still recomputes them from `round7-A-lcov.info`. *(Round 9 is the full pass this
section was waiting for: §10.1 clears the blocker, §1 carries the new numbers — 2392 / 96 / 280 —
and the record now cites `round9-A-lcov.info` at `b50efdb55`.)*

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

**Status (round 9): both overflows are gone on this tree — see §10.1 for the two runs that
reproduce round 8's filters and finish green, and §1.1 for the two full passes that followed.** The
park in `agent_bridge::import` is still unexplained and did not recur in round 9's two passes; it
stays recorded here rather than retired.

## 10. Round 9 — the overflow is gone, the lane measures itself again, and the two blocks that were left

### 10.1 The blocker: `remote_peer`'s stack overflow **no longer reproduces**

Round 8 could not take a whole-crate pass (§9.4). The first thing this round did was re-check that
claim **on the current tree**, before writing any test, because a blocker that has silently gone is
worse than one that is named — every later round would keep paying for it:

| run | filter | command | result |
|---|---|---|---|
| `round9-peer-check.log` | `remote_peer::` | `cargo test -p future-host --lib -- "remote_peer::"` | **131 passed / 0 failed / 211.42 s, rc 0 — no overflow** |
| `round9-verify-e2e-check.log` | `remote::verify_e2e` | `cargo test -p future-host --lib -- "remote::verify_e2e"` | **20 passed / 0 failed / 4 ignored / 5.45 s, rc 0 — no overflow** |

Both are the exact filters that produced `thread 'tokio-rt-worker' has overflowed its stack` in
round 8's logs, under the same env (`HOME` at the worktree's `target/test-home`, `ulimit -n 8192`,
toolchain 1.99.1). The verdict is therefore **"已不复现" / "does not reproduce"**, not "fixed here":
this round changed no product code and no thread-stack setting, and the tree it measured contains
the other lanes' work that landed in `dev-huichen` between `deb03ec4a` and `b50efdb55`. The honest
statement is the measurement: *at `b50efdb55`, both filters run to completion, so the pass is
available again.* Whether the cause was one of those landings or the machine's transient state is
not established here, and nothing in this lane depends on the answer — the two full passes in §1.1
are the evidence that matters.

The blocker is recorded here as one row rather than fixed in this round, which was the instruction:
a stack overflow in a **tokio worker** is not a fixture fault that a test-only change should paper
over. If it returns, the three logs to compare against are the two above plus round 8's
`round8-*-baseline.log`.

### 10.2 `remote_host/pairing.rs` — 22 open → 1, in six new tests

Five of the six drive real HTTP against the scripted platform the sweep's success path already uses
(`MockPlatform`); the sixth injects a filesystem failure. What each one pins is the **refusal**,
because in this file the refusals are what keep a live code alive:

| test | lines | what it pins |
|---|---|---|
| `a_sweep_that_cannot_list_devices_leaves_the_refusal_alone` | 490-492 | a platform that cannot list this desktop's devices leaves the cap refusal as the answer: **no delete** from a list that never arrived, and no second mint |
| `a_stale_invitation_the_platform_will_not_delete_survives` | 503-505, 540-541 | the list arrives and the delete is refused: one attempt at the row, the refusal named, and the cap refusal still stands |
| `removing_the_last_slot_that_cannot_be_unlinked_is_an_error` | 292-293 | a directory where the legacy credential file belongs makes `remove_file` fail, and the caller is told by name instead of being told the unpair worked |
| `a_relay_room_that_cannot_be_deleted_is_logged_and_never_queued` | 770-776 | a self-hosted room's delete is attempted detached, its refusal is logged, and **nothing is queued** for a pairing that has no account |
| `an_invitation_survives_a_bookkeeping_failure` | 831-833 | a pending-revokes path that is a directory fails the retire step, and the invitation is still handed back with the platform's own code |
| — | 1682 | not covered by a new test: the `RelayEnv` restore arm, whose coverage is decided by test order (§1.2), stays open |

Four lines added by these tests are §3 W20 (two `match` catch-alls and two message arguments).
`pairing.rs:372` moves to §3 as W19: it is the closing error edge of `PairingIdentity::new`, which
fails only when the OS RNG fails.

### 10.3 `agent_host.rs` — 29 open → 26, at the entry's gate

This file's residual is mostly process-wide one-way state and fixture-harness arms, which is why
only its gate was closable. Two tests, both state-only fixtures in the style of the file's existing
`recovery_declines_every_state_it_must_not_repair`:

| test | lines | what it pins |
|---|---|---|
| `a_start_mode_the_entry_does_not_understand_is_refused_before_any_probe` | 363 | `FUTURE_DESKTOP_AGENT_MODE` is the operator's word on what a launch may do; a value the entry does not understand is refused **before** any probe, so a typo neither starts an embedded service nor attaches to whatever answered |
| `a_local_start_permission_is_honoured_only_while_the_host_is_still_starting` | 435, 443 | a probe with nothing listening hands back a `LocalStartPermit`; attach-only mode refuses it outright, and a host that stopped while the probe ran takes nothing. The fixture asserts it reached `AgentSelection::LocalStart`, so the test cannot pass by any other route |

What is left (26) is named row by row in §4; the two biggest are `is_stopping()`'s arms (144, 177 —
closing admission is one-way for the whole test binary) and the fixture harness's own failure arms.

### 10.4 Falsification, per assertion

Eight mutations, eight red runs, the tree restored after each (`round9-falsify.log`, driven by
`falsify9.py` / `falsify9b.py`):

| test | mutation | result |
|---|---|---|
| `a_sweep_that_cannot_list_devices…` | `if cleared.is_empty()` → `&& false` (retry a mint that freed nothing) | red on the `Remote { status: 409 }` assertion |
| `a_stale_invitation_the_platform_will_not_delete…` | `delete_device` treats every status as success | red on the same assertion |
| `removing_the_last_slot_that_cannot_be_unlinked…` | the unlink's `map_err(…)?` → `.ok()` | red: `unwrap_err` on an `Ok` |
| `a_relay_room_that_cannot_be_deleted…` | the relay branch no longer returns early (falls through to queueing) | red on `!pending.exists()` |
| `an_invitation_survives_a_bookkeeping_failure` | the log arm → `result.expect(…)` | red: the invitation call panics |
| `a_start_mode_the_entry_does_not_understand…` | the mode guard → `if false` | red: `select` answers `Ok` |
| `a_local_start_permission…` (attach-only) | the refusal's sentence changed by one space | red on the exact-equality assertion |
| `a_local_start_permission…` (overtaken) | the phase check answers `Err` instead of `Ok` | red: `run_selected` no longer returns `Ok` |

The one mutation deliberately **not** used: dropping `mode == "external"` so the guard falls
through, which would have taken a real embedded service up under the fixture. The sentence mutation
is the weaker falsifier for that half and is named as such.

### 10.5 What this round did not do

- No product code changed. The two blocks are tests only; the one fixture-adjacent change is
  `crate::remote::test_support::HomeGuard` (held by the `agent_host` gate tests to serialize against
  anything that publishes `HOME`), which already exists.
- `cli_tool.rs`'s 22 open lines are the cli lane's, not read here (§4).
- No `#[ignore]`, no retry and no widened timeout: the two `select` tests use a probe against an
  endpoint nothing listens on, so they fail rather than wait.

### 10.6 After the merge: the tip reads **2394**, and the two lines that are new

Round 9's A/B pair is measured at `b50efdb55`. That commit is on `dev-huichen` now (landed as the
merge `4ae1f3f0a`, 21:51), but before that the branch tip `120e3d511` merged `dev-huichen` into
this branch, and that merge rewrote **`host/src/remote/test_support.rs`** — `+53` lines at 1085,
`+5` at 1146, `+58` at 1198 and `+50` at 2585, the `FakeNats` publish record (`PublishLog`) and the
tests that drive it — plus `remote/tests.rs`'s `runtime_tests`. So the record's commit stopped
being the tip's tree: the suite went **1969 → 1982** tests, and three of the 58 `DA:0` lines the
record lists for that file (1963, 2193, 2710 — §3 W7) now sit *past* the insertions.

A probe at the tip, same recipe, whole lib, no filter (`round9-tip-A.sh` → `round9-tip-A-run.log`,
`round9-tip-A-lcov.info`, `round9-tip-A-missed.json`): **1971 passed / 0 failed / 11 ignored in
460.94 s**, `report rc=0`. Round 9's own seven tests were also re-run on it directly and are green
(`agent_host::tests` **24 passed**, `remote_host::pairing::http_tests` **36 passed**), so the merge
did not disturb what this round added.

| | record (`round9-A`, `b50efdb55`) | tip probe (`round9-tip-A`, `120e3d511`) |
|---|---:|---:|
| lane total `DA:0` | 2392 | **2394** (+2) |
| files with any drift | — | **1** — `remote/test_support.rs`, 58 → 60 |
| tests in the suite | 1969 | 1982 |

The drift is **three renumbered waivers and two new lines**, and the difference matters:

| line at the tip | what it is | why it reads 0 |
|---|---|---|
| `1119` | `entries.pop_front()` — the eviction arm of `PublishLog::record` | it runs only once the record is full: `RECORDED_PUBLISHES` is **2048**, and the fixture's heaviest test injects **300** (`for index in 0..300` in `the_publish_record_keeps_a_packet_the_tap_ring_dropped`). The guard on 1118 executed **4423** times and the push on 1120 **4423** times; the body never. Reachable, not reached — a test that publishes 2049 times covers it |
| `2602` | the `matches!(` inside that same test's `assert!` | macro-expansion attribution, not a statement: the assert executes (2601 reads **1**) and its argument line 2603 reads **1**, while the opening line of the expansion carries a region with 0. Same family as §3's attribution rows |
| §3 W7: 1963 → **2078**, 2193 → **2308**, 2710 → **2875** | `violations.push(…)`, its `join` in the assert message, `Err(error) => panic!("expected the deferred arm…")` | nothing was covered: the same three failure arms simply moved. Matched **by content**, which is why the per-file diff reads "closed 3 / added 5" — arithmetic, not a claim |

The buckets therefore do **not** close at the tip: 2016 switch lane + 96 waived + 280 open = 2392,
and the remaining two lines are unassigned — either would be an ordinary §4 row or a §3
attribution row, but assigning one without a re-measured pair would be a number typed rather than
measured.

What this means for the next reader, stated so the wrong number is not quoted: `host-lane.json`
keeps `measurement_commit: b50efdb55` and its **2392**, because that pair is what
`lane-check.py` validates and A and B agree line for line (§1.1); **the tree the campaign continues
on reads 2394 at `120e3d511`** = 2392 + those two, which the next round puts in their buckets as
part of a re-measured pair. No product code and no test changed for this probe — it is a
measurement, so the two lines stay exactly where the report put them.

## 11. Round 11 — the two blocks it ate, the pass that re-measured them, and a test that wrote the process environment

### 11.1 What round 11 measured

Three whole-`--lib` passes, all green: the base probe (`d3b2c58fd`, **2394**), the pair A/B on the
tree that carries the round's two new blocks (`c6b7b4979`, **2375** / **2377**, §1.1), and pass C
on the tree that carries §11.7's seam (`0bf7f96d9`, **2378**, §1.1.1). **1971 tests** at the base
probe and **1979** in all three passes after it, 11 `#[ignore]`d, 0 failed in every one (the eight
new suite entries are round 11's four `agent_host` cases plus the `trust` side's three assertions
and the child entry they run). Against the base probe the
lane closes **17 in-scope lines** — six in `agent_host.rs`, eleven in `remote/trust.rs` — and one
§1.2 mover (`agent_host.rs:502` → `565`) returns to `DA:0`, which is the `−16` §1 explains.

### 11.2 `agent_host.rs` — six lines, four arms, one process each

`5f7aa615e` added four `block_fixtures` cases, each a re-exec of the test binary through
`run_case` (`--exact agent_host::block_fixtures::host_fixture`, its own `FUTURE_HOME` and its own
agent socket):

| case | arm it drives | lines it closed |
|---|---|---|
| `recover-stopping` | `close_admission()` then `recover`: the refusal must come by name, and nothing may be repaired | `144` |
| `recover-unfinished` | an owned service that never publishes `finished` inside the stop window: the incomplete-cleanup flag is recorded | `173-174` |
| `cancelled-start` | a worker that answers without taking ownership or reaching Ready: the launcher is told the start was cancelled instead of waiting out its deadline | `334` |
| `owned-idle` | a live handle's idle admission is closed for an update, and refuses a new command | `216-217` |

A process per case is the only shape that works, not a preference: `close_admission()` is one-way
for the whole process, so a fixture that reached it would answer every later test on that binary the
same way — the leak class `remote::test_support::HomeGuard` exists to prevent for `HOME`, here
avoided by never sharing the process at all.

After §11.7's seam, the rows this row owns move by whole lines: `177` → `243`, `311-313` →
`377-379`, `451` → `514`, `487/503/531/539/665` → `550/566/594/602/728`, and the fixture harness's
own failure arms `762-767, 815, 890, 1027-1028` → `858-860, 862-863, 911, 1046, 1183-1184`.

### 11.3 `remote/trust.rs` — 17 → 6, eleven lines

`5f7aa615e` added one-process-per-construction fixtures for the operator relay CA
(`a_relay_ca_file_that_holds_no_certificate_is_refused_by_name`,
`…_that_cannot_be_read_is_refused_by_name`,
`a_bundle_that_cannot_be_written_leaves_the_platform_load_in_place`), and `c6b7b4979` gave the
three cases one builder (`bundle_fixture` + `run_case` + `stderr`) and removed the unknown-case
`unreachable!()`. Eleven lines closed: the `bundle_with_extra` refusal arms (`64`, `66`, `68`,
`70-71`, `73`, `75`, `91-93`) and the trailing-newline guard (`87`).

Why a process each: `bundle_with_extra`'s arms sit behind a `OnceLock`, so one closure execution
per process — a single-process test can only ever take the first arm it reaches.

What is left is `load_platform_certs`'s error collection (`169-174`): the platform store's read has
to fail, which needs a `SSL_CERT_FILE` (or equivalent) that this lane cannot publish under
`TEST_HOME_LOCK` without deciding every other test's TLS. That is §4's row, not a waiver.

### 11.4 Falsification: five mutations, four flipped, one did not

`round11-falsify.sh` mutates one line at a time and runs the single test that is supposed to depend
on it (`round11-falsify.log`; `round11-falsify-run.log` records the restore). This is where
"covered" is separated from "pinned":

| # | mutation | filter | result |
|---|---|---|---|
| M1 | the bundle stops appending its final newline (`trust.rs:87`) | `an_operator_relay_ca_…` | **rc=101, FAILED** — the guard is pinned |
| M2 | `recover`'s first `is_stopping()` guard stops refusing (`agent_host.rs:143`) | `a_recovery_attempt_during_shutdown_…` | **rc=0, still green — the mutation survives** |
| M3 | an unfinished cleanup is not recorded (`agent_host.rs:173`) | `a_recovery_whose_cleanup_cannot_finish_…` | **rc=101, FAILED** |
| M4 | a worker that took nothing reports a timeout (`agent_host.rs:334`) | `a_worker_that_takes_nothing_…` | **rc=101, FAILED** |
| M5 | reserving idle admission answers false (`agent_host.rs:216-217`) | `an_owned_ready_agent_reserves_…` | **rc=101, FAILED** |

**M2 is the one this file will not overclaim.** The fixture cannot see the difference: with the
first guard removed, `recover` walks on into its closure, whose own `is_stopping()` check answers
the same sentence, leaves `phase` where it was, and the assertion `assert_eq!(error, "Desktop is
stopping")` still passes. The test's comment says the second guard is why a `contains` cannot tell
them apart — the same symmetry is why deleting the *first* guard cannot be seen from here either.
`144` is therefore recorded as **executed**, not as **pinned by a mutation**: the report's `DA>0` is
a measurement of the call, and this table is the measurement of the claim.

What would distinguish them, and the next check to try: give the fixture `owned: true`. With the
guard in place `recover` returns before `owns_agent()`; without it the closure enters
`stop_until(now + STOP_TIMEOUT)`, so either the elapsed time or the phase after the call differs.
Until that fixture exists, `144` belongs in §4's mechanism column with this caveat, which is where
the row now points instead of counting it as a closure on the strength of an execution alone.

### 11.5 What this round did not do

- No product code changed in round 11. Both blocks are tests; the one product-code change in this
  worktree is §11.7's seam, and it is a `#[cfg]`-gated read that a product build cannot arm.
- `cli_tool.rs`'s 22 open lines are the cli lane's, not read here (§4).
- No `#[ignore]`, no retry, no widened timeout: the `agent_host` cases use fixtures with their own
  deadlines, and a case that hangs fails rather than waits.
- §4's 267 open lines stay open. Nothing in this round was waived to make the number smaller.

### 11.6 W3 retired, and the waiver total that follows

§3's W3 claimed `remote/trust.rs:96-98` as ghost lines of the crate's second (dependency)
compilation. It does not reproduce: those three lines read **>0** in the base probe and in every
pass since (`1`, `4`, `4`, and >0 in C). The row is retired in place with a zero line count rather
than deleted, so a reader can see what it used to claim, and the waiver total is **95** — 76 in
named rows plus 19 closing edges.

### 11.7 The start-mode test wrote the process environment

`a_start_mode_the_entry_does_not_understand_is_refused_before_any_probe` needs `select` to read a
mode the entry cannot honour. It did that with `std::env::set_var("FUTURE_DESKTOP_AGENT_MODE",
"embedded")` and a restore afterwards, and that is wrong in a way that does not show up in a green
run: the environment is **process-global**, libtest runs this binary's tests in parallel threads,
and the window is exactly the one this value decides — any other test's `select` during it would
refuse to start, and `environment::start_logging` logs the same variable. It is the leak class
`docs/testing/flaky-tauri.md` item 27 records and the reason `HomeGuard` serializes its `HOME` swap
on `TEST_HOME_LOCK`.

What replaced it: `select` reads through `start_mode()` → `launch_mode()`, and `launch_mode()`
consults a **thread-local** `START_MODE_OVERRIDE` before the environment. `StartModeGuard::set`
arms it and clears it on drop, so a panicking test cannot hand its mode to the next test on the same
thread — the shape `cli_tool::PROBE_DEADLINE_OVERRIDE` already uses for the probe deadline. The
gate is `#[cfg(any(test, feature = "test-support"))]` for the same reason as that one: an entry
crate compiles this crate as a dependency of its own test binary, where `cfg(test)` is false. An
unarmed slot falls through to the environment, so product behaviour is unchanged —
`cargo check -p future-host --features test-support` builds that form, and `select` still refuses
before touching the probe or the host state.

The test was falsified against the seam rather than against ambient state: with
`StartModeGuard::set("auto")` it fails at `unwrap_err()` on an `Ok`, which is what proves the guard
— not a stray environment value — is what it measures. It also no longer needs `HomeGuard`: nothing
on this path resolves `HOME` or opens a socket, so the test holds no fixture lock at all.

The measurement question the edit raised is answered by pass C (§1.1.1): the seam's lines are all
covered, and the only rows that changed value are the three §1.2 movers.
