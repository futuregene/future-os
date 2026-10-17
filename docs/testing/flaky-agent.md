# Flake workstream — `future-agent` (units F-AGENT, then F-AGENT-2)

Four items, three different outcomes — one mechanism **already removed upstream** (and now
pinned by an ordering assertion), one **fixed by construction** with a mutation control, and
two **measured but not reproducible here**, one of which needed a fix outside F-AGENT's
declared write set. That fix was handed to **F-AGENT-2**, which applied it under a granted
scope extension (that one file) and verified it with a deterministic control (§3.4) rather
than with a rate alone — the useful outcome when the host cannot show the flake at all.

| item | where (verified line) | verdict |
|---|---|---|
| cli_smoke profile-timer races (FLAKES.md item 3) | `agent/tests/cli_smoke.rs:230`, `:344` | **mechanism already removed** by commit `22d4d42bc` (#1108, the readiness barrier); `0/10` before, `0/20` after; the bind-before-timer order is now an explicit assertion; negative control reproduces the bind-failure signature |
| `a_transport_error_is_classified_by_the_kind_that_caused_it` (item 4) | `agent/src/llm/mod.rs:957` | **fixed in this unit** — the catch-all input is constructed from the predicate table instead of a loopback `302` handshake; `0/10 + 0/20` before, `0/20 + 0/20` after; mutation control fails at the same assertion |
| `timeout_drain_handoff_executes_queued_command` (§5b item 13) | `agent/src/session/persistence.rs:1660` | **not reproduced** in 12 full lib-suite runs (load 8–17) + 40 module runs (load 50–77); the two wall-clock assumptions are named below; the fix was outside this unit's write set — **patch handed off and applied by F-AGENT-2** (§3.3–§3.6): the drain flag is armed before `barrier()`, the 200 ms margin is a bounded wait, and a 50 ms-delay control fails **6/6** on the old order and passes **5/5 + 5/5** on the new one |
| `doctor_detects_agent_binary_not_running_as_issue` (§5b item 14) | `cli/src/commands/doctor.rs:960` | **out of scope (`cli/`) and already fixed by F-CLI-2** (`docs/testing/flaky-cli.md` §7.1); independently re-measured here `0/20` quiet + `0/20` at 48 burners + `0/4` doctor-module runs at 32 burners |
| `compat.rs:235` covered/uncovered oscillation (§5b item 16) | `orchestration/goal/src/compat.rs` | measurement hazard, not this unit's scope; not touched |

## 0. Run identity and environment

| field | value |
|---|---|
| unit / agent id | F-AGENT (`todo_cd63021f6d76`) |
| worktree | `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`, base `8449acf6`) |
| host | macOS (darwin, arm64), 32 CPUs |
| load while measuring | plain windows **4–17**; hostile windows **13–77** (own burners, see each section); F-AGENT-2's own windows are quoted per batch in §3.5 and in the row below |
| F-AGENT-2 rate windows (load, per batch) | suite `-j 4` 11:41–11:49: 1-min load **4.8 → 6.9**; module + own 64 burners 11:49–11:51: **13 → 67** |
| contention during F-AGENT-2 | **another unit was compiling beside it.** REV-TAURI-SHELL was dispatched at the same time and ran `cargo-llvm-cov` in its own scratch checkout `/private/tmp/rev-tauri-shell-s10-88746` with a private `--target-dir`. Verified by `ps -o lstart`: started **11:50:17**, so it overlapped the **last ~1 min of the module window only** and **none of the suite window** (11:41–11:49). It sat at **0.0% CPU with no `rustc` children** for its whole life, so it contributed ≈no CPU; the **13 → 67** rise in the module window is this unit's own 64 burners. The supervisor recorded the co-dispatch as its own scheduling mistake (PLAN §7.1); it is recorded here so a reader knows every rate was taken on a shared host, not a quiet one. |
| files changed | F-AGENT: `agent/src/llm/mod.rs`, `agent/tests/cli_smoke.rs`; F-AGENT-2: `agent/src/session/persistence.rs` (**test-only**, the §3.3 hunk); both units: this file. **Nothing under `cli/`** — the `PATH` finding is reported, not edited (§4) |
| follow-up unit | F-AGENT-2 (`todo_a88d7c598ac3`); scope extension granted for `agent/src/session/persistence.rs` (this ticket only) |
| F-AGENT-2 environment | same worktree/branch; private `CARGO_TARGET_DIR=target/cov-f-agent-persist`, isolated `HOME=$PWD/target/test-home`, `ulimit -n 10240`, rustup 1.97.0 toolchain, `-j 4` for every cargo invocation |

Every shell used the PLAN §2 environment:

```bash
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home          # the real ~/.future must not leak into tests
mkdir -p "$HOME"
```

Flake work needs no instrumentation (FLAKES.md §2.3), so everything below ran with plain
`cargo test` against the shared `target/debug`. Driver for every rate:
`.future/cov100/flake-rate.sh --label <name> --runs N --timeout T -- <cmd>`; logs are under
`/tmp/cov100-flake-agent/flake-<label>/` (outside the repo). The target dir was never
`cargo llvm-cov`-instrumented, so no number here is a coverage measurement.

**Hostile windows** are the unit's own burners (`( while :; do :; done ) &` ×N), started,
held for the whole window, and killed afterwards. Before/after windows for the same item
are adjacent in time and at the same burner count; the 1-min load is quoted per window.

## 1. Item 3 — `cli_smoke` profile-timer races

Tests (current names; the brief and FLAKES.md cite lines that have since drifted):

* `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint` — `agent/tests/cli_smoke.rs:230`
* `agent_starts_serves_and_shuts_down_via_profile_timer` — `agent/tests/cli_smoke.rs:344`

Both run the real binary with `--profile-seconds 0`.

### 1.1 Protocol step 1 — reproduce on the unmodified tree

```bash
flake-rate.sh --label cli-smoke-before --runs 10 --timeout 300 \
  -- cargo test -p future-agent --test cli_smoke -j 4
# runs=10 failures=0 timeouts=0   (load 3–5; ~7 s/run; logs /tmp/cov100-flake-agent/flake-cli-smoke-before)
```

It does **not** reproduce. That is expected on this revision, and the reason is in the
source, not in the scheduler.

### 1.2 The mechanism was already removed at `22d4d42bc` (#1108)

The recorded mechanism is: `--profile-seconds 0` lets the profile timer win the race
against the bind log line the test asserts on. On the current tree that race is gone by
construction:

* `agent/src/cli.rs` `async_main` creates `running.serve(Some(ready_tx))` and then, before
  any shut-down source exists, awaits readiness with `tokio::select! { biased; result =
  &mut server => …, ready = ready_rx => … }` (`agent/src/cli.rs:541-547`, with the comment
  *“A zero-duration timer can run on another worker before `serve` is first polled. Admit
  the listener before enabling host shutdown sources”*).
* The server's first poll logs `gRPC server listening on …` and only then lets `serve` send
  the readiness identity (`agent/src/grpc/mod.rs:91`, `agent/src/service/mod.rs:296-361`).
* Only **after** `ready_rx` resolves is the profile-timer task spawned and the zero-duration
  `sleep` started (`agent/src/cli.rs:548-582`).

So the bind line is written before the timer task exists; the timer cannot win. This was
introduced by `22d4d42bc` (“harden embedded Agent ownership and update recovery”, PR #1108,
2026-10-16), i.e. before this ticket was written — the brief's line numbers (`:258`, `:345`)
and its “mechanism is a race in the TEST” premise both predate that fix.

I did **not** rebuild the pre-`22d4d42bc` binary to obtain a failing “before” rate: the
production file is outside this unit's write set, and the prior CI evidence already carries
that number (FLAKES.md item 3 / GOTCHAS.md: the graceful shutdown sequence present, the bind
line absent). The honest statement is: **on this tree the rate is 0; the code path that
produced the old failures no longer exists.**

### 1.3 What changed in the tests, and why it is not cosmetic

A passing rate alone would let a future change re-arm the timer before the listener is
admitted and silently reintroduce the race. Both tests now assert the ordering contract
they depend on:

* the bind line must be **present** (test 2 previously asserted no bind line at all), and
* its byte offset in the captured log must be **before** `Profile timer expired`.

Nothing was deleted or weakened; the existing assertions (exit 0, the local-IPC arm, the
socket path, the completed shutdown sequence, the flamegraph file) all remain.

`Some(1)` with a completed shutdown sequence was test 2's recorded failure signature; the
new assertion fails with the whole log embedded, so that signature can no longer be
confused with a transport regression.

### 1.4 Negative control — the assertions still fire when the bind really fails

Occupy the instance's own local endpoint, then run the same binary with the same
`--home … --profile-seconds 0`:

```bash
H=$(mktemp -d); mkdir -p "$H/run"
python3 -c 'import socket,sys,time; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.listen(1); time.sleep(30)' "$H/run/agent.sock" &
HOME=$H USERPROFILE=$H target/debug/future-agent --home "$H" --profile-seconds 0 --verbose
# EXIT=1
# "gRPC server listening on local IPC" occurrences: 0
# stderr: Agent exited with error: local endpoint is active or cannot be classified as stale: …/run/agent.sock
```

So `the default transport must be local IPC` (and the new ordering assertion) fail when the
listener was not admitted — the control cannot pass vacuously. The suite's existing
`agent_exits_nonzero_when_grpc_bind_fails` (cli_smoke.rs:536) is the TCP-side counterpart.

### 1.5 After

```bash
flake-rate.sh --label cli-smoke-after --runs 20 --timeout 300 \
  -- cargo test -p future-agent --test cli_smoke -j 4
# runs=20 failures=0 timeouts=0   (load 14–25; logs /tmp/cov100-flake-agent/flake-cli-smoke-after)
```

Neighbourhood: the full agent lib suite after the change — `2424 passed; 0 failed;
1 ignored` (42 s).

## 2. Item 4 — `a_transport_error_is_classified_by_the_kind_that_caused_it`

`agent/src/llm/mod.rs:957` (brief said `:1057`).

### 2.1 The mechanism

The test's last block constructed an error *outside* every named transport kind by having a
loopback socket answer `302 Found` to a client whose redirect policy rejects the hop. Under
load the handshake could be reset; the error then arrived as `is_request()`, so
`reqwest_stream_error_kind` returned `"request"` and the assertion *“the catch-all arm needs
an error outside every named kind”* failed with the exact `reqwest::Error { kind: Request,
… hyper::Error(Io, ConnectionReset) }` recorded in CI.

### 2.2 Protocol step 1 — reproduction attempts (all negative on this host)

| rig | runs | failures |
|---|---|---|
| focused test, quiet (load 4–5) — `cargo test -p future-agent --lib -- llm::tests::a_transport_error_is_classified_by_the_kind_that_caused_it` | 10 | 0 |
| same command, own 32 burners | 20 | 0 |
| the test binary directly, own 64 burners (`target/debug/deps/future_agent-* --exact …`) | 500 | 0 |

The socket exchange is a few hundred microseconds on this 32-core host; the CI failure
needs a saturated 2-core runner. The flake is therefore recorded as **not reproduced here**;
the fix below does not depend on reproducing it, because it removes the handshake from the
path under test rather than making the handshake faster.

### 2.3 The fix — construct the out-of-kind input instead of racing for it

`reqwest::Error` has no public constructor, and the kinds outside the five predicates
(`Builder`, `Redirect`, `Status`) are produced only inside reqwest; the only way to obtain
one from a real exchange is a live request. So the classifier's decision table is now a
pure function:

```rust
fn transport_kind_from_predicates(
    is_timeout: bool, is_connect: bool, is_body: bool,
    is_decode: bool, is_request: bool,
) -> &'static str { /* ordered arms, else "unknown" */ }
```

and `reqwest_stream_error_kind(&reqwest::Error)` is a three-line delegation passing the five
predicates. Production behaviour is unchanged (the predicates are pure and side-effect-free;
the only difference is that all five are now evaluated instead of short-circuiting).

The test then pins every table row — including **all five false → `"unknown"`** — with
constructed booleans, and keeps a **real** reqwest error outside every named kind: the
unbuildable-URL `Builder` error it already constructed, now also asserted with
`error.is_builder()`. The loopback `302` block and its `server.join()` are gone; it covered
only the same fallback line that the builder error covers, and it was the sole source of
the race. Nothing else in the test changed: the `connect`, three `decode`, and real-error
table checks still run against sockets and real errors.

### 2.4 Mutation control — the assertion can still fail

Temporarily changing the fallback arm to return `"request"` (scratch edit, reverted
immediately) and running the same command:

```
thread 'llm::tests::a_transport_error_is_classified_by_the_kind_that_caused_it' panicked at agent/src/llm/mod.rs:1091:9:
assertion `left == right` failed
  left: "request"
 right: "unknown"
test result: FAILED. 0 passed; 1 failed
```

So the new assertion is not vacuous; re-introducing a guessed fallback fails it.

### 2.5 After

| rig | runs | failures |
|---|---|---|
| focused test, quiet | 20 | 0 |
| focused test, own 48 burners (load 13–19) | 20 | 0 |

Commands are the two in §2.2 with `--runs 20`; logs
`/tmp/cov100-flake-agent/flake-llm-kind-after-quiet` and `…-after-load48`.

`cargo clippy -p future-agent --all-targets -j 4 -- -D warnings` → exit 0; `cargo fmt
--check -- agent/src/llm/mod.rs agent/tests/cli_smoke.rs` → exit 0. Neighbourhood: full lib
suite 2424 passed / 0 failed, cli_smoke 25 passed / 0 failed.

## 3. §5b item 13 — `timeout_drain_handoff_executes_queued_command`

`agent/src/session/persistence.rs:1660` (brief said `:1587`). **Not fixed by F-AGENT: the file
was outside that unit's declared write set** (`agent/tests`, `agent/src/llm`,
`docs/testing/flaky-agent.md`). Measured and diagnosed there; the patch was left in §3.3 and
**applied by F-AGENT-2**, which was granted `agent/src/session/persistence.rs` for this
ticket. §3.4 is the deterministic control, §3.5 the post-fix rate, §3.6 the gates.

### 3.1 Reproduction attempts

| rig | runs | load | failures |
|---|---|---|---|
| full lib suite — `cargo test -p future-agent --lib` (the reported condition) | 12 (6+6, same command) | 8–17 | 0 |
| `session::persistence::tests` module at default 32-way parallelism, own 96 burners | 40 | 50–77 | 0 |
| the single test binary directly, own 64 burners | 200 | 18–44 | 0 |

Logs: `/tmp/cov100-flake-agent/flake-persist-full-1`, `…-full-2`,
`…-persist-module-load2`, `…/direct-persist.log`. One earlier `persist-module-load` batch
(40/40 “FAIL”) is **not** a datum: the command placed `-j 4` after `--`, libtest rejected it
(`error: Unrecognized option: 'j'`) — the PLAN §8c shape, caught before recording.

### 3.2 The mechanism visible in the test (two wall-clock assumptions)

```rust
let persistence = SessionPersistence::with_idle_timeout(…, Duration::from_millis(20));
persistence.append(…).unwrap();
persistence.barrier().unwrap();          // worker is now idle in recv_timeout(20 ms)
persistence.drain_on_timeout_for_test(); // ← armed AFTER the worker can already time out
std::thread::sleep(Duration::from_millis(200));   // ← fixed wall-clock margin
assert!(persistence.inner.worker.lock().is_none());
assert!(loaded.entries.iter().any(|e| entry_text_starts_with(e, "drained")));
```

Two independent ways the outcome can depend on the host:

1. **Arming order.** The worker loops into `recv_timeout(20 ms)` as soon as the barrier ack
   is sent. The test thread must then return from `barrier()`, be scheduled, and execute the
   `drain_on_timeout` store. If it is descheduled for >20 ms, the worker times out with the
   flag still false, finds an empty queue, and **retires** — the injected `drained` append
   never happens, so the *second* assertion fails while the first passes.
2. **Fixed margin.** Even when the handoff lands, retirement needs a second 20 ms timeout
   plus two `execute` calls; `sleep(200 ms)` is a 5× wall-clock margin. Under instrumented
   or heavily loaded runs a starved worker thread can exceed it, and the *first* assertion
   (`worker.is_none()`) fails.

Both are the same defect family as the sibling test immediately above it
(`stolen_worker_slot_is_not_retired_by_foreign_worker`, which was already converted to a
bounded condition wait: `for _ in 0..1000 { if … { break } sleep(5 ms) }`).

### 3.3 The patch (applied by F-AGENT-2)

```diff
         persistence
             .append(vec![SessionEntry::new_user(
                 "user",
                 serde_json::json!("hi"),
             )])
             .unwrap();
-        persistence.barrier().unwrap();
+        // Arm the handoff BEFORE the worker can observe an idle timeout. The
+        // worker re-enters recv_timeout(20 ms) as soon as the barrier ack is
+        // sent; arming after `barrier()` races that timeout, and a starved test
+        // thread loses the race: the worker retires with an empty handoff and
+        // the drained append never happens.
         persistence.drain_on_timeout_for_test();
+        persistence.barrier().unwrap();
         // The next idle timeout finds the injected append in the drain window,
         // executes it, and only then retires.
-        std::thread::sleep(Duration::from_millis(200));
+        for _ in 0..1000 {
+            if persistence.inner.worker.lock().is_none() {
+                break;
+            }
+            std::thread::sleep(Duration::from_millis(5));
+        }
         assert!(persistence.inner.worker.lock().is_none());
```

Note the ordering subtlety: the flag must be armed **before the worker's first idle timeout
after the last command** — concretely, before `barrier()` is sent, not merely before the
sleep, because the worker re-enters `recv_timeout` the moment it acknowledges the barrier. The
diff below arms it after `append` (which starts the worker) and before `barrier()`; arming it
before `append` would also be safe — the worker does not exist until `append` starts it, so
the flag is merely set earlier — but the diff's position is already the earliest that matters.
With this order the
handoff lands in the first timeout deterministically, and retirement follows at the next one;
the bounded wait replaces the wall-clock margin. The drain branch and its `try_recv` handoff
execute exactly as before (no coverage change).

**Verification owed by the file's owner at handoff, and now done:** apply, re-run the 12-run
full-suite rate and a hostile module rig, and run the deterministic control the paragraph
above proposes — a 50 ms sleep at the old arming point must fail the *current* order and pass
the *fixed* order. That is §3.4.

One design note, recorded because it is the only place the applied code could have differed:
arming after `append` but before `barrier` (the author's diff) is sufficient, and is what was
applied. Arming before `append` would also be safe — a worker that retires in that gap is
replaced by `barrier()`'s `ensure_worker_locked`, which then sees the already-armed flag —
but the diff was applied verbatim rather than "improved". Both statements were checked against
`try_send`/`ensure_worker_locked` and `run_worker`'s timeout arm before applying.

### 3.4 Deterministic control — the old order fails under the same 50 ms hazard, the new one passes

This host cannot reproduce the descheduling race (32 cores; F-AGENT measured 0/252 across
quiet, 96-burner and direct-binary rigs, and the post-fix rate in §3.5 is also 0/N). So the
fix is verified by **inducing the hazard**: a 50 ms `sleep` stands in for "the test thread was
descheduled longer than the 20 ms idle timeout", inserted at the point the diagnosis names.
Each control is a scratch edit, reverted before the next; the final tree is §3.3 verbatim
(`git diff` shows only that hunk).

| control | test order | 50 ms delay inserted | runs | outcome |
|---|---|---|---|---|
| **A — old order** | `barrier()` then arm | between `barrier()` and `drain_on_timeout_for_test()` (the old arming point) | 6 | **FAIL 6/6**, always at the *same* assertion: `panicked at agent/src/session/persistence.rs:1682: assertion failed: loaded.entries.iter().any(|entry| entry_text_starts_with(entry, "drained"))` — and `assert!(worker.is_none())` passed, i.e. for the wrong reason. (`:1682` is the control-A build; the same assertion is at `:1693` in the patched tree.) |
| **B — new order** | arm then `barrier()` | immediately before the arm (the corresponding point after reordering) | 5 | **PASS 5/5** |
| **C — new order** | arm then `barrier()` | after `barrier()` (the literal old position) | 5 | **PASS 5/5** — the old hazard point is now harmless, because the arm no longer follows the barrier |
| **final** | arm then `barrier()`, no delay | — | 5 focus + 12 suite + 40 module | PASS |

What this establishes, and why it is stronger than a rate:

* control A is the §3.2 mechanism **observed**, not argued — the worker retires inside the
delay, the flag is armed too late, and the failure is the *second* assertion while the *first*
passes, exactly the "fails for the wrong reason" signature;
* control B carries the *same* hazard and still passes, so the fix removes the dependence on
the 20 ms timeout rather than widening a margin — the anti-pattern this campaign rejected in
F-TAURI-3 (its withdrawn tolerance);
* control C shows the specific window is closed rather than moved: delaying the test thread
*after* the barrier is now irrelevant because the arm already happened.

The assertion is not vacuous: A is a live negative control on the very assertion the fix
depends on. The `try_recv` handoff arm still executes (the `drained` entry it produces is what
the second assertion reads), so the patch is test-ordering only: no new uncovered lines, and
the drain branch is exercised exactly as before.

### 3.5 Rate after — same command, same `-j 4`, load stated

| rig | runs | load (1-min, start → end) | failures |
|---|---|---|---|
| full lib suite — `cargo test -p future-agent --lib -j 4` | 12 | 4.8 → 6.9 | **0** |
| `session::persistence::tests` module, own 64 burners | 40 | 13 → 67 | **0** |

Logs: `/var/folders/…/T/flake-persist-full-after/` (flake-rate default) and
`/tmp/cov100-flake-fagent2/logs-module/`. The burners were removed after the window
(`stop-burners.sh --apply`: 64/64 pids matched the burner shape, 0 left alive). A module run is
26 tests (`2399 filtered out`); a suite run is `2424 passed / 0 failed / 1 ignored`.

**The rate did not improve, and is not claimed to have.** After = 0/52, before = 0/52; a
32-core host does not deschedule this thread for >20 ms even at load 67, which is the same
reason the pre-fix rigs measured 0. The rate is reported as a non-regression. The claim the
fix rests on is §3.4.

### 3.6 Gates (F-AGENT-2)

| gate | command | result |
|---|---|---|
| fmt | `cargo fmt --check -- agent/src/session/persistence.rs` | exit 0 |
| clippy | `cargo clippy -p future-agent --all-targets -j 4 -- -D warnings` | exit 0, **0** warnings/errors |
| full lib suite | `cargo test -p future-agent --lib -j 4` | **2424 passed, 0 failed, 1 ignored** (the ignored count is unchanged from the pre-patch run) |

## 4. §5b item 14 — `doctor_detects_agent_binary_not_running_as_issue`

`cli/src/commands/doctor.rs:960` (brief said `:815`). **Out of this unit's scope (`cli/`).**
Diagnosed and re-measured only; no file under `cli/` was edited.

The FLAKES.md row already records this as **fixed by F-CLI-2 (2026-10-16)**, and the fix is
in the source this tree carries: `ProbeBudgetGuard::set(None)` at `doctor.rs:999` removes the
5 s wall-clock deadline from the capture test (the kill arm keeps the production budget in
`binary_version_kills_a_probe_that_never_exits`), `PATH` is narrowed to
`<fake-bin>:/usr/bin:/bin`, `FUTURE_AGENT_GRPC_ADDR=127.0.0.1:1` isolates it from any agent
on the host, and the two formerly identical arms (`VersionProbe::{NoVersion, NotStarted}`)
are now distinguishable. That record's own targeted rig was 2/20 → 0/9
(`docs/testing/flaky-cli.md` §7.1). The supervisor's steering (“host-state race with peers'
agents up”) refers to the pre-fix diagnosis; the current test pins the address away from any
peer agent.

Independent re-measurement on this tree:

| rig | runs | load | failures |
|---|---|---|---|
| `cargo test -p future-cli --lib -j 4 -- commands::doctor::tests::doctor_detects_agent_binary_not_running_as_issue` | 20 | 5–8 | 0 |
| same command, own 48 burners | 20 | 11–14 | 0 |
| the whole `commands::doctor::tests` module (26 tests), own 32 burners | 4 | 8–35 | 0 |
| the whole `commands::doctor::tests` module, quiet (F-AGENT-2, 11:55) | 1 | 3.3 → 2.9 | 0 — **26 passed / 0 failed** in 38 s |
| the whole `-p future-cli` lib suite (1166 tests), quiet (F-AGENT-2, 11:56) | 1 | 2.8 | 0 — **1166 passed / 0 failed** in 109 s |

Logs: `/tmp/cov100-flake-agent/flake-doctor-quiet`, `…-doctor-load48`,
`…-doctor-module-load32`.

**Conclusion for the supervisor: no fix is needed in `cli/` for this item; it is already
fixed and now independently measured at 0/44.** If a future failure reappears, the first
check is the residual the F-CLI-2 record states: a child that cannot be *spawned* at all
(EAGAIN/ENOMEM under load) still fails the capture, and is now labelled `NotStarted(…)`
instead of mimicking “no version”.

One out-of-scope observation made while reading the file (not edited; re-verified by F-AGENT-2
at the line the follow-up ticket cites): this test sets the process-global `PATH` with a bare
`std::env::set_var` at **`cli/src/commands/doctor.rs:985`** — while the *same function* routes
`HOME` and `FUTURE_AGENT_GRPC_ADDR` through `EnvGuard::set`, so the gap is internal to the
test, not missing infrastructure. Nothing restores `PATH`; that is the item-27 shape, harmless
here only because the narrowed PATH still contains `/usr/bin` and `/bin`, so a later test that
needs a host binary happens to keep working. Two further bare `set_var("PATH", …)` calls sit
at `:1049` (`doctor_captures_partial_version_from_hanging_binary`) and `:1548`
(`doctor_reports_an_unreachable_agent_with_a_binary_on_path`) — all three verified present and
all three unguarded. `cli/` is outside this unit's scope: **reported, not touched**; the
file's owner should convert all three to the existing `EnvGuard`, whose `Drop` restores the
saved value (`cli/src/test_env.rs:58-68`).

**Is it live?** No — measured, not assumed. With the leak present, `commands::doctor::tests`
is 26/26 green and the **whole `-p future-cli` lib suite is 1166/1166 green** (load 2.8,
11:56), so no test in that process depends on the process-global `PATH` after the leak. The
hazard is that the next test which does (anything outside `/usr/bin:/bin`, e.g. the rustup
toolchain) inherits a `PATH` pointing into a **deleted tempdir**. That is a latent trap, not
a current flake, which is why it is P2 and why nothing was changed here without a grant.

**Patch prepared, not applied** (`/tmp/doctor-path-envguard.git.patch`, exact `git apply`
form): three hunks, each `std::env::set_var("PATH", …)` → `let _path = EnvGuard::set(&[("PATH", …)])`;
no production line changes; the patched copy is `rustfmt --check`-clean, so the only remaining
step is `cargo test -p future-cli --lib -j 4 -- commands::doctor::tests`.

**DECISION REQUESTED (blocking `todo_ed01692d14ad`, which is parked `blocked`):** a one-line
grant of `cli/src/commands/doctor.rs` for this test-only edit. The ticket body says *
only report it, do not touch it, unless I widen the scope again*", and neither the steering nor
any ruling in this turn widens it, so nothing was written under `cli/`.

## 5. §5b item 16 — `orchestration/goal/src/compat.rs:235`

A covered/uncovered oscillation (scheduling race) that changes any before/after coverage
number around it. Not this unit's scope and not a test flake to fix; recorded here only so
the measurement hazard is not attributed to this unit's files.

## 6. Summary of rates (same command, same `-j`, adjacent windows)

| item | before | after |
|---|---|---|
| cli_smoke suite (`-j 4`) | **0/10** (load 3–5) | **0/20** (load 14–25) |
| llm classification test | **0/10** quiet, **0/20** @32 burners, **0/500** direct @64 | **0/20** quiet, **0/20** @48 burners |
| persistence test, full lib suite (`-j 4`) | **0/12** (load 8–17) | **0/12** (load 4.8–6.9, F-AGENT-2) |
| persistence module | **0/40** @96 burners (load 50–77) | **0/40** @64 burners (load 13→67, F-AGENT-2) |
| persistence, induced 50 ms hazard | **FAIL 6/6** (old order) | **PASS 5/5 + 5/5** (new order) |
| doctor test (cli/) | **0/20** quiet, **0/20** @48 burners, **0/4** module @32 | unchanged (already fixed by F-CLI-2) |

What is claimed, and what is not:

* **Claimed:** the llm test's catch-all input no longer depends on a network handshake
  (mutation-controlled); the cli_smoke ordering contract is now asserted, and its old
  mechanism is absent from the current source; the persistence item's two wall-clock
  assumptions are removed — the drain is armed before the barrier and the 200 ms margin is a
  bounded condition wait — and that is proven by a control in which the **old order fails
  6/6** and the **new order passes 10/10** under the same induced 50 ms hazard.
* **Not claimed:** a better observed rate anywhere. No persistence failure was ever seen on
  this host (0/52 before, 0/52 after), so the fix rests on construction + control; the
  cli_smoke “before” is the pre-#1108 code, not a rebuild of it.
* **Left open:** the `PATH` item-27 shape in `cli/src/commands/doctor.rs` (`:985`, `:1049`,
  `:1548`) — reported only, because `cli/` is outside both units' scopes.
