# The cli lane's gate flake: a process started inside a fixed window

**Status: conclusion, 2026-11-01.** Written because this lane's gate cannot be
green-or-red on its own numbers while one unrelated test decides it. It records
what was observed, what was measured, what is *ruled out*, and the one thing
still missing. No product code and no timeout was changed for it (the coverage
todo forbids a timeout in this lane).

Lane: `cargo llvm-cov -p future-cli`, the measurement in
[`cli.md`](cli.md). Evidence files are named per claim, under
`coverage/2026-11/after/cli/`.

## 1. What was observed

`browser::safari::safari_manager::tests::launch_then_become_reachable_reports_started`
failed inside a full instrumented `--lib` run, at the test's **own** 20 s wait for
the driver script's marker:

```
run12.log:1094   test browser::safari::safari_manager::tests::launch_then_become_reachable_reports_started ... FAILED
run12.log:2536   panicked at cli/src/browser/safari/safari_manager.rs:500:13:
                 the driver was never spawned, so the poll was never reached
run12.log:2543   test result: FAILED. 1531 passed; 1 failed; ... finished in 172.77s
run12-lib-retry.log:1760  test result: ok. 1532 passed; 0 failed; ... 101.13s
```

Four full `--lib` runs were made in wave 12: two green, two red on one or two
tests from that module. The module alone is **20/20 green** (`cargo test -p
future-cli --lib -- browser::safari::safari_manager`, 21.77 s), so the trigger is
the full run, not the code path.

The same red run carried **21** `has been running for over 60 seconds` lines —
20 of them `commands::doctor::tests` / `commands::init::tests`, which all queue on
`test_env::lock_env()`, plus `a_healthy_generation_resets_the_budget_and_a_retryable_reconnect_arms_the_retry`.
Those are *lock-queue* delays, not hangs: they are the measurement that the host
was starved badly enough for per-test wall clocks to move by minutes.

The neighbouring family tells the same story on a different fixture. `run3.log`
(three failures, all `headless::tests`) and `run.log` (one) failed like this:

```
the fixture CLI must be accepted by discovery: CLI …/bin/future: CLI discovery timed out.
Install a matching CLI beside Desktop or correct PATH; no fallback CLI was executed.
```

i.e. a script the fixture wrote, launched by the product, did not answer inside
the product's probe budget.

## 2. What was measured this wave (and did not reproduce)

`safari-load-repro.sh` (in this directory) runs the full `--lib` with burners and
kills them on every exit path. Five loaded rounds, all green:

| round | configuration | load (1 min) | result |
|---|---|---|---|
| 1-3 | `--test-threads=8` + 8 burners | 5.2 → 12.3 | 1532 passed, 0 failed (106 s, 94 s, 140 s) |
| 4-5 | default parallelism (32) + 12 burners | 23.1, 45.0 | 1532 passed, 0 failed (91 s, 95 s) |

Logs: `safari-load-r1…r5.log`. Round 3 was the only one with slow tests (3), and
rounds 4-5 — at the *highest* load — had none and were the fastest.

So "thread count" and "CPU burners alone" are **not** the trigger at this size.

**But a plain, uninstrumented full `--lib` run did fail, on both launch tests at
once** (the wave-13 gate run, `gate-lib-r1…r3.log`):

| run | build | result |
|---|---|---|
| `gate-lib-r1` (lost) | plain `cargo test -p future-cli --lib` | **FAILED** — `launch_then_become_reachable_reports_started` **and** `start_launch_success_against_fake_driver`, 1541 passed / 2 failed, 141.14 s |
| `gate-lib-r1` (re-run) | identical | 1543 passed, 0 failed, 125.00 s |
| `gate-lib-r2`, `r3` | identical | 1543 passed, 0 failed, 80.49 s / 99.27 s |

Two facts fall out of that, and both matter more than the rate:

* The failing run was the **slowest** plain run (141 s against 80-125 s), i.e. the
  host was busy with something other than this crate — the other sessions on this
  machine. Instrumentation is therefore *not* required for the flake, only a
  loaded host.
* **Two tests failing together** rules out the hypotheses that are per-test
  (below): a foreign listener on one test's port, or one test's reserved port
  being stolen, cannot do this. What both tests share is that each asks the host
  to start a new process (`/bin/sh` driver scripts) and each then waits, and the
  only shared mutable state they touch is guarded by the same env lock.

**The message from that red run was lost**: the command's output was piped through
`grep` instead of captured to a file, so the `started.is_finished()` line the wave
added — the very thing that would have named the arm — never reached a record.
That is a self-inflicted loss worth writing down: capture first, filter second.


## 3. Which arm can produce that message — each one forced

`safari_start` can return before the marker exists only along these paths. Each
was measured, not reasoned about:

| # | arm | how it was forced | observed |
|---|---|---|---|
| a | **platform gate** (`!is_macos()` → `Err`) | replaced the test's `force_macos_gate()` with `Some(false)` for one run | `safari_start failed before spawning a driver: Safari is only available on macOS.` in **0.04 s** |
| b | **fd exhaustion at spawn** (`Command::spawn` → EAGAIN/EMFILE) | ran the built test binary at `ulimit -n 64` and `96` | 1 passed each time (1.02 s / 0.51 s) — not reproducible by shrinking the soft limit to this level, and the lane's shell default is 256 |
| c | **the poll window** (product's 10 s budget) | — | **not this message**: this test already runs under `ReadinessBudgetGuard::set(200)`, so its poll is a probe count, and a poll failure reports `a driver that answers in the window is a success`, not the marker wait |
| d | **another process on the port** (`endpoint_reachable` → `already_running`, then a refused session) | — | reachable in principle: `reserved_port()` guarantees uniqueness *within* one process only, and this host runs several sessions' `cargo test` at once; unforced |
| e | **`/bin/sh` starved past 20 s** | — | unforced; the marker is written by the driver script itself, so this needs the host to delay a `fork`+`exec` of `/bin/sh` by 20 s |

(a) is ruled out as the cause: it is serialized by `test_env::lock_env()`, and it
now prints a different sentence than the one observed. (c) is ruled out by the
test's own construction. (b) did not reproduce at a limit this large — though the
arm exists and is the only one that is *plainly* load-coupled. (d) and (e) are
the two survivors, and they are indistinguishable from the old failure message:
both leave the marker absent and both are external-process/timing dependent.

## 4. The one thing this wave added: the failure can now name itself

`launch_then_become_reachable_reports_started` reported its own 20 s wait while
the product's answer was thrown away. Its sibling
(`start_launch_success_against_fake_driver`) has checked `started.is_finished()`
since its driver-port record replaced a sleep; this test never did. Wave 13
ported that check (test code only, no product change):

```
if started.is_finished() {
    match started.await {
        Ok(Ok(_))   => panic!("safari_start answered without spawning a driver"),
        Ok(Err(e))  => panic!("safari_start failed before spawning a driver: {e}"),
        Err(join)   => panic!("the safari_start task panicked: {join}"),
    }
}
```

The forced gate run above is its positive control: the same arms now answer in
0.04 s with the product's own words. **Falsifier**: before the change the same
input printed the marker-wait sentence — which is exactly how wave 12 produced
`the driver was never spawned, so the poll was never reached`.

## 5. Judgment, in the flake workstream's form

* **Mechanism.** A test whose verdict depends on an *external process* starting
  and answering inside a fixed wall-clock bound. Two such bounds sit in this
  test's path (the product's 10 s readiness window, now a probe budget; the
  test's own 20 s wait for the driver's marker), and one such bound sits in the
  `headless::tests` discovery probes that failed in the same loaded runs. Under a
  starved host the bound, not the code, decides — the same finding
  `docs/testing/flaky-cli.md` §2 and §7.3 made for the two neighbouring browser
  tests, whose fixes are "bring the endpoint up from the *test* process, after a
  record the flow itself produces" and "a probe count, not a clock".
* **Positive control.** Forced platform refusal → product's reason in 0.04 s
  (above). For the *load* arm, none: five loaded rounds and three plain full runs
  stayed green, so this wave has no fail-then-pass pair for (d)/(e) — the red run
  is a single event whose message was lost.
* **Counter-evidence (what argues against the obvious story).** More threads and
  more burners made the run *faster* (rounds 4-5) and produced no slow tests at
  all, so "thread count" is not the trigger; and the one plain red was the
  *slowest* plain run, so the trigger is a host busy with work this crate does not
  own (this machine runs several sessions at once) rather than the run's own
  parallelism. What survives is the class, not the arm.
* **What is not in hand.** A red run **with** the new diagnostic captured to a
  file. That single log line separates (d)/(e) from (b) and closes this item;
  without it the class is established and the arm is not. (§7 below is six more
  runs with the diagnostic in place, at higher load, and not one red — and the
  arm was identified by an injected control on another branch, `0900e710c`.)

## 6. Handed to the flake goal, not fixed here

This lane's rules forbid adding a timeout, a retry or a weakened assertion to
make its own gate green, and forbids a product change made for a test. All three
would have been the cheapest route to a passing gate (a 60 s marker wait, a
`--test-threads=1` run, or a `#[ignore]`), and all three are refused. What the
lane keeps is the measurement (`run12.log`, `run12-lib-retry.log`,
`safari-load-r1…r5.log`), the reproduction (`safari-load-repro.sh`), the forced
controls above, and one test-side diagnostic.

Next useful check, in order: (0) **capture every run to a file** — `cargo test …
> log 2>&1`, never a pipe — because the one red this wave saw only has value if
its `started.is_finished()` line survives; (1) re-run the full `--lib` until it
goes red (1 in 11 runs today — four plain, five loaded, two instrumented — and
worse when the host is busy), read the new line and close the arm; (2) if (d) is
what it says, give the launch test the `$2`-recording idiom its sibling already
has, so a foreign listener on the requested port can no longer be mistaken for
the driver; (3) if (e), the fix is in `safari_start`'s own ordering, not in the
test.

## Appendix — the reproduction, inline

`coverage/` is gitignored, so the driver is repeated here (it is the script that
produced `safari-load-r1…r5.log`). Burners are killed on every exit path;
`--test-threads` and the burner count are arguments so the two configurations in
§2 can be compared.

```sh
#!/bin/sh
# ./safari-load.sh [rounds] [burners] [extra cargo-test args...]
set -u
ROUNDS=${1:-3}; BURNERS=${2:-8}; shift 2 || true
cd "$(git rev-parse --show-toplevel)/.worktrees/cov-cli-new" || exit 1
export PATH=$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH
unset CARGO_BUILD_TARGET CARGO_TARGET_DIR
export HOME=$PWD/target/test-home
OUT=$(cd ../../../coverage 2>/dev/null; pwd)/2026-11/after/cli
PIDS=""
i=0; while [ "$i" -lt "$BURNERS" ]; do yes > /dev/null & PIDS="$PIDS $!"; i=$((i+1)); done
trap 'for p in $PIDS; do kill "$p" 2>/dev/null; done' EXIT INT TERM
sleep 2; r=0
while [ "$r" -lt "$ROUNDS" ]; do
    r=$((r+1))
    echo "=== round $r $(date +%H:%M:%S) load=$(sysctl -n vm.loadavg)" | tee -a "$OUT/safari-load.log"
    cargo test -p future-cli --lib -- "$@" > "$OUT/safari-load-r$r.log" 2>&1
    echo "rc=$? slow=$(grep -c 'has been running' "$OUT/safari-load-r$r.log")" >> "$OUT/safari-load.log"
    grep -E "^test result|panicked at|safari_start (failed|answered)" "$OUT/safari-load-r$r.log" >> "$OUT/safari-load.log"
done
```

For the instrumented configuration the same loop uses
`cargo llvm-cov -p future-cli --no-report` instead of `cargo test`, which is the
only shape that has produced a red so far.


## 7. Six more runs with the diagnostic in place — and the arm was (e) (wave 14, 2026-11-01)

§5's "what is not in hand" was a red run **with** the `started.is_finished()` line captured to
a file. Wave 14 went after it and did not get one. Every run was captured to a file
(`coverage/2026-11/after/cli/flake-hunt.sh`, never a pipe):

| run | build | load average (1 min, at start) | result |
|---|---|---|---|
| `run14.log` | instrumented `cargo llvm-cov -p future-cli` | 38.8 | 1543 passed / 0 failed, 102.66 s |
| `flake-hunt-rounds1-3.log` r1 | plain `cargo test -p future-cli --lib`, 8 burners | 61.8 | 1543 / 0, 97.33 s |
| r2 | same | 80.6 | 1543 / 0, **198.01 s** |
| r3 | same | 63.1 | 1543 / 0, 110.97 s |
| `flake-hunt.log` r1 | same | 40.6 | 1543 / 0, 81.28 s |
| r2 | same | 45.8 | 1543 / 0, 89.04 s |

Two facts from those runs:

* The loads were **higher** than the run that produced wave 13's red (which was the slowest of
  its three, 141 s) and one run took **198 s** — slower than that red — and was green. So
  "the slowest run decides" is not the mechanism at this size; whatever it is, it is rarer
  than the load alone.
* Zero reds in six runs is consistent with §5's 1-in-11 estimate (≈0.57 expected).

### It closed in the same window, on another branch

While this wave was running, `0900e710c` ("a safari launch test is not decided by its own
clock", the flake workstream) landed on `dev-huichen` and is in the base this wave's final
measurement is taken against. It **removes the 20 s marker wait** from both launch tests: the
wait now ends when the fixture's record appears *or* when the launch task finishes, and its
only bound is the product's probe budget — a count, not a duration. Its injected control is
the arm identification §5 lacked: a fake driver that sleeps **25 s** before touching its
marker is **2/2 red** with the old shape (both tests, both with exactly "the driver was never
spawned, so the poll was never reached") and green with the new one. So the arm was **(e)** —
`/bin/sh` starved past the test's own clock — and never **(d)**, a foreign listener. The item
is closed by that fix; this section's six green runs are the un-injected baseline that says
the ambient rate was low enough to hide behind, not evidence of a remaining defect.

**Deliberately not done: porting the `$2` recording to this test.** §6's step (2) made that
conditional on a red *naming a foreign listener*, and no red ever named one — the injected
control above names (e) instead. The sibling's idiom
(`start_launch_success_against_fake_driver` records the port the CLI resolved and serves the
mock there) would also add its own external-process wait — the very construction this record
is about — and test-side `DA:0` lines (the shape wave 13's diagnostic added five of). Porting
it would have been a change made for a hypothesis the evidence has since refuted.
