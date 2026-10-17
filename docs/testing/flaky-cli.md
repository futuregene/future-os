# Flake workstream — `future-cli` (unit F-CLI)

Three items, three different outcomes — one fixed with a deterministic before/after, one
shown to be **already fixed upstream** with a measured negative control, and one
**reproduced and diagnosed but not fixed**, because the fix lies outside this unit's
declared write set (`cli/tests`, `cli/src/browser`, `docs/testing/flaky-cli.md`).

| item | where | verdict |
|---|---|---|
| agent singleton-lock contention (FLAKES.md item 5) | `cli/tests/bin.rs` | already fixed upstream (commit `ebda57c6`, PR #353) — `0/20` runs; mechanism re-injected → `12/20` |
| safari readiness window (FLAKES.md item 6, todo (b)) | `cli/src/browser/safari/safari_manager.rs` | **fixed** in this unit; deterministic negative control fails before / passes after; `10/10` clean runs |
| doctor version probe (FLAKES.md §5b item 14, todo (c)) | `cli/src/commands/doctor.rs:815` (assertion `:852`), sibling `:892` | **reproduced (2/4 `cargo test -p future-cli` attempts, 2/8 loaded full-suite rounds, 2/20 doctor-only rounds), mechanism named, fix handed off** — the file is outside this unit's write set |

## 0. Run identity and environment

| field | value |
|---|---|
| unit / agent id | F-CLI (`worker-flake-cli`) |
| worktree | `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`, base `8449acf6`) |
| host | macOS (darwin, arm64), 32 CPUs; load average **5–14** while measuring (13 campaign workers share the host0) |
| my change under test | `cli/src/browser/safari/safari_manager.rs` (the converted launch test) |

Every shell used:

```bash
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home          # the real ~/.future must not leak into tests
mkdir -p "$HOME"
```

**Target directory:** flake work needs no instrumentation (`FLAKES.md` §2.3), so
everything below ran against the *shared* `target/debug` with plain `cargo test`, as
`FLAKES.md` §2.3 prescribes. I did not create a private `target/cov-flake-cli`: this
crate's test targets link the whole embedded workspace, and `target/debug` is already
57 GB on a volume with 54 GB free (`df -h .` → 95 % capacity), so a second uninstrumented
tree is the expensive way to obtain the same binary. Nothing here was measured under
`cargo llvm-cov`.

Driver for the rate loops: `.future/cov100/flake-rate.sh --label <name> --runs N --timeout T -- <cmd>`.

## 1. Item (a) — `cli/tests/bin.rs` agent singleton-lock contention

### 1.1 Protocol step 1: reproduce on the unmodified tree → **it does not reproduce**

```bash
flake-rate.sh --label bin-after --runs 20 --timeout 300 \
  -- cargo test -p future-cli --test bin -j 4
# runs=20 failures=0 timeouts=1   (logs /tmp/flake-cli-bin-after)
```

The single `TIMEOUT` is run 1 and is a *harness* artifact, not a test datum: the log
shows the timed-out attempt never ran a test (cargo was waiting on the build/lock under
load). Every run in which the binary ran was `16 passed; 0 failed`.

### 1.2 The mechanism, reproduced directly

The user-level singleton lock is real production behaviour. With one agent holding it,
a second agent on the same FutureOS home exits **1 with completely empty stdout** and
this stderr:

```
$ HOME=/tmp/lockdemo ./target/debug/future agent --grpc-addr 127.0.0.1:52999 &   # holds the lock
$ HOME=/tmp/lockdemo ./target/debug/future agent --grpc-addr 127.0.0.1:52998
rc=1
stdout: []
stderr: [Error: Future Agent is already running for this FutureOS home (lock: /tmp/lockdemo/.future/agent/agent-instance.lock) …]
```

That is exactly the documented CI symptom of `embedded_agent_run_failure_is_exit_1`
(exit code 1 — the expected one — with empty stdout, so the assertion message is the
bare `stdout: `).

### 1.3 Negative control: re-injecting the pre-fix construction

Pre-fix, `cli/tests/bin.rs` spawned its agents with the *real* HOME, so the tests (and
the developer's running agent) contend on one lock. I reproduced that construction
temporarily, in a form that cannot touch the real `~/.future`: `future_with_home`
ignored its per-test home and used one *shared scratch* home for the whole test binary.

```bash
# temporary edit, working tree restored afterwards (see §6)
flake-rate.sh --label bin-negative-control --runs 20 --timeout 120 \
  -- target/debug/deps/bin-d9af8c972ef5cd2f
# runs=20 failures=20 timeouts=0
#   embedded_agent_run_failure_is_exit_1                     12 appearances
#   init_runs_its_command_and_reports_a_failed_install        20 appearances
# first panic: embedded_agent_run_failure_is_exit_1 -> cli/tests/bin.rs:140
#              assertion `left == right` failed / "stdout: "   (empty stdout)
```

`embedded_agent_run_failure_is_exit_1` reproduces the documented failure **12/20** with
its documented message. (`init_runs_its_command_and_reports_a_failed_install` failing
20/20 is an artifact of my crude patch — sharing one home breaks that test's premise of
a clean home; it is not part of the flake and not evidence about it.)

### 1.4 The fix is already in the tree

`future_with_home` (spawn every agent with its own temp `HOME` **and** `USERPROFILE`)
landed in `ebda57c6` — *"test(cli): isolate HOME for agent-spawning bin.rs tests (kill
singleton-lock flake) (#353)"* — which is an ancestor of the campaign base. Checked on
`origin/main` itself:

```bash
git show origin/main:cli/tests/bin.rs | grep -c 'future_with_home('   # 6
git diff origin/main -- cli/tests/bin.rs                              # empty
```

Rates, same command, same host:

| construction | command | runs | failures |
|---|---|---|---|
| current tree (per-test HOME) | `cargo test -p future-cli --test bin -j 4` | 20 | **0** |
| pre-fix construction (shared home), negative control | `bin-d9af8c972ef5cd2f` | 20 | **20** (`embedded_agent_run_failure_is_exit_1` 12) |
| pre-fix construction with the *real* HOME (spot check) | `cargo test -p future-cli --test bin -j 4` | 1 | 1 (same `stdout: ` panic) |

**Verdict:** no code change from this unit for item (a) — the cause is already fixed, and
the fix is verified here by a measured negative control. One observation for the
reviewer, not a request: `embedded_agent_logfile_failure_returns_err` asserts only
`stderr.contains("Error:")`, which the singleton-lock rejection also satisfies, so that
test on its own cannot distinguish "logfile init failed" from "lost the lock" — the
isolation is what makes its outcome deterministic, not the assertion.

## 2. Item (b) — the safari 10 s readiness window: fixed

### 2.1 What was wrong

`start_launch_success_against_fake_driver` (`cli/src/browser/safari/safari_manager.rs`)
had the driver itself serve the port: a `#!/bin/sh` script that `exec python3`'d a tiny
HTTP server, while the CLI's launch path polls for readiness for **10 s**
(`safari_manager.rs:141-162`) and then fails with `safaridriver did not respond at …`;
the test carried a comment that the python interpreter's page-in "can otherwise exceed
the 10 s readiness window" and a `python3 --version` pre-warm that does not import the
modules the server needs. So the test's outcome is decided by the host's process start-up,
not by the code under test — the same shape as the profile-timer races in
`agent/tests/cli_smoke.rs`.

### 2.2 Reproducing it by forcing the host condition (it is not luck-reproducible)

Protocol step 1 first: the test alone passes (`0/10` failures in §2.4), so a "rate"
alone cannot demonstrate it. The mechanism is made explicit with a `python3` shim first
on `PATH` that sleeps 12 s before `exec`ing the real interpreter — i.e. an interpreter
that cannot possibly be up inside the CLI's window:

```bash
# /tmp/slowpy/python3:  #!/bin/sh ; sleep 12 ; exec /opt/homebrew/bin/python3 "$@"
$ PATH=/tmp/slowpy:$PATH target/debug/deps/future_cli-88d8c1d19c3818d9 \
      start_launch_success_against_fake_driver
```

| revision | with the 12 s-slow interpreter | result |
|---|---|---|
| old fixture (python driver) | **FAIL 1/1** | `panicked at cli/src/browser/safari/safari_manager.rs:816: start: "safaridriver did not respond at http://127.0.0.1:51532 within 10s."` (22.8 s) |
| converted fixture | **pass** | `1 passed` (0.51 s) |

A fail-then-pass pair on the *same forcing input* is stronger than two small rates: it
shows the removal is causal, not a coincidence of load.

### 2.3 The fix

The launch test now uses the file's own deterministic idiom (`launch_then_become_reachable_reports_started`
already used it for the same reason) and adds one thing that test does not do:

* the driver script is `#!/bin/sh`, **records the port the CLI resolved** (`$2` — a
  `#!` script needs no interpreter *modules*, and nothing else is on the critical path),
  then `exec sleep 20` so the process it reports is a live one this test reaps with the
  existing `KillChild`;
* the test waits for that record, then brings the WebDriver endpoint up **from the test
  process** (`serve_webdriver`) on the port read from the record;
* assertions kept and extended: `status == "started"` (not the already-running branch),
  the resolved port, `launcher == SAFARIDRIVER_PATH`, `protocol() == "webdriver"`,
  `session_id == Some("sid-start")`, `driver_pid` is a live pid, exactly one
  `POST /session`, only `/status` + `/session` ever asked, and the poll did see
  `/status`.

Reading the resolved port from the driver (rather than assuming `free_port()`'s number)
also removes a second host dependency the sibling test still has: `resolve_port`
(`safari_manager.rs:253-269`) scans upward when the requested port is taken, so a port
grabbed by another process between `free_port()` and `resolve_port` pointed the CLI's
poll at a port nobody served. That is not hypothetical — it is what the A-3 run below
caught in the **sibling** test:

| run (loaded full-suite, 2 concurrent streams) | failing test | message |
|---|---|---|
| A-3 | `launch_then_become_reachable_reports_started` (the *sibling*, unmodified) | `panicked at safari_manager.rs:430: a driver that answers in the window is a success: "safaridriver did not respond at http://127.0.0.1:65381 within 10s."` |

Reported rather than "fixed": that test is not in this unit's change set; the same
`$2`-recording idiom (or any construction that does not assume the resolved port equals
the requested one) is the fix if its owner wants one.

### 2.4 Verification

```bash
flake-rate.sh --label safari-after --runs 10 --timeout 120 \
  -- target/debug/deps/future_cli-88d8c1d19c3818d9 start_launch_success_against_fake_driver
# runs=10 failures=0 timeouts=0

cargo test -p future-cli --lib -j 4 -- browser::safari::safari_manager
# 19 passed; 0 failed   (the whole module, neighbourhood included)
```

Not a production change: the CLI's 10 s readiness budget is product behaviour and stays
covered by `start_launch_never_ready_times_out` (asserts `>= 10 s` **and** the message)
and by the Windows twin `launch_failure_and_readiness_timeout_are_both_reported`.

**Residual exposure, stated plainly.** The production readiness loop is still a
wall-clock budget (`safari_manager.rs:141-162`), so a host that starves *this test's own
process* for 10 s between the record appearing and the mock's `bind` can still fail the
test. What is gone is every dependence on a *separate* process getting started inside
that budget (the interpreter) and on a port number taken for granted; what remains is the
product's own budget versus a starved test process. The same residual is visible in this
campaign in a peer-owned test (§5, third bullet).

## 3. Item (c) — `doctor_detects_agent_binary_not_running_as_issue` (steering item 14)

### 3.1 Reproduction

Command (the binary is the same one `cargo test -p future-cli --lib` runs; two copies in
parallel, which is what the coverage unit's batch looked like with peers active):

```bash
target/debug/deps/future_cli-88d8c1d19c3818d9 --test-threads=32     # x2 concurrent streams, 4 rounds each
```

| round | result | failing |
|---|---|---|
| A-1 / B-1 | ok 1159/1159, 120 s each | — |
| **A-2 / B-2** | FAILED 1157/1159, **402 s / 411 s** | `doctor_detects_agent_binary_not_running_as_issue`, `doctor_captures_partial_version_from_hanging_binary` |
| A-3 | FAILED 1158/1159, 147 s | `launch_then_become_reachable_reports_started` (safari sibling, §2.3) |
| B-3 / A-4 / B-4 | ok | — |

So the doctor flake reproduces **2/8 loaded full-suite rounds**, **2/4 plain
`cargo test -p future-cli` attempts** (§4) and **2/20 doctor-only rounds** — and it needs
saturation: the failing full-suite rounds were 3.4× slower than the clean ones.

Narrower rig — *only* the doctor tests, four concurrent streams of
`commands::doctor::tests --test-threads=19`, 5 rounds each: **2/20 rounds failed**, in
both cases the same single test and the same assertion. Under light load it does not
reproduce at all: 6 concurrent single-test streams × 40 runs (240 runs) → **0 failures**;
the 23-test module alone → 0 failures.

### 3.2 The failing assertion — and the documented hypothesis, refuted

```
thread '…doctor_detects_agent_binary_not_running_as_issue' panicked at cli/src/commands/doctor.rs:852:9:
stdout: Future Doctor — checking environment...
[--] Login      Not logged in — run `future auth login`
[!!] Agent
      /var/folders/…/.tmpzzw1qq/fake-bin/future-agent          <-- no "(future-agent v0.0.1)"
      Connection check: unable to connect to Future Agent (http://127.0.0.1:1: transport error)
      Agent unavailable — open Desktop or start `future agent`; check compatibility and instance locks
[ok] CLI        /opt/homebrew/bin/future  (future v0.0.2-8449acf6+local)
[ok] Desktop    /opt/homebrew/bin/future-desktop  (Unknown option: --version)
```

Line 852 is the test's **third** assertion, `assert!(stdout.contains("v0.0.1"), "stdout: {stdout}")`.
The `[!!]` section and `Agent unavailable` — the first two assertions — are **present**.
The clue that was attributed to "peers' agents being up" (`.future/cov100/FLAKES.md` §5b
item 14, quoted from the cli coverage unit's journal) is therefore not what fails: the
doctor did report the agent as an issue. What is lost is the **version captured from the
`#!/bin/sh` fixture**, while the two probes of *Mach-O* binaries in the same run kept
theirs.

The sibling site fails identically, in the same rounds: `doctor_captures_partial_version_from_hanging_binary`
(assertion at `doctor.rs:892`) — `expected partial --version output to survive the
timeout`, with its `#!/bin/sh` `future-desktop` line printed without the captured text.

### 3.3 Mechanism (named) and what I could not split

`get_binary_version` (`doctor.rs:227-290`) mirrors Node's
`execFile(…, {timeout: 5000})`: `.stdout(piped)` + two drain tasks, a **5 s wall-clock
deadline from spawn**, then `SIGKILL` + reap; output that has not been written by the
deadline is gone. For a `#!` script the probe additionally depends on the kernel
starting the interpreter and the interpreter reading and running the script *inside that
window*. Baseline cost of exactly that on this host, measured while the machine was
loaded (load ≈ 10): `n=60 avg 34 ms max 186 ms` (`/tmp/shbench.sh`, a `#!/bin/sh` script
that echoes one line, fork+exec+echo timed). So the *generic* cost is two orders of
magnitude below the budget; the samples that fail are the tail of a saturated machine
(the failing rounds ran 2–3.4× slower overall), where the child has produced nothing by
the deadline and is killed.

Two candidate sub-mechanisms remain distinguishable only from inside the probe, and the
test's output renders them identically (a missing version either way):

* the `#!` child was starved past the deadline (deadline arm fired), or
* `Command::spawn` failed and `get_binary_version` returned `None` via `.ok()?`
  (`doctor.rs:233-239`) — the same "an error to *start* the probe is reported as
  *no version*" shape as `which()` below.

I could not split them: the failure needs sustained saturation (2/8, 2/20 rounds), a
bisect needs instrumentation inside `doctor.rs`, and `cli/src/commands/doctor.rs` is
**outside this unit's declared write set** (`cli/tests`, `cli/src/browser`,
`docs/testing/flaky-cli.md`), so I made no out-of-scope write.

A third host dependency of the same tests is visible without any load at all: the two
tests keep the *real* `PATH`, so on a developer machine they probe and **spawn the
host's own binaries** — the failing output above literally shows
`/opt/homebrew/bin/future` and `/opt/homebrew/bin/future-desktop` (a GUI binary; the
"don't let a GUI binary in a test reach the Tauri plugin stage" hazard in
`.future/cov100/GOTCHAS.md`). `doctor_clean_environment_output` already sets an isolated
`PATH`; these two do not.

### 3.4 A production weakness the diagnosis exposed (separate decision)

`which()` (`cli/src/utils/files.rs:87-93`) maps **any** failure to run the lookup
command (`Command::new("which"|"where").output().await.ok()?`) to `None`, i.e. "not
installed", and `check_agent` (`doctor.rs:356-368`) then reports the missing binary as a
**Warn** (`Standalone future-agent is optional …`) instead of the Issue. Demonstrated
causally against the real binary, with `which` shadowed so the lookup yields nothing:

```bash
env -i HOME=/tmp/docprobe/home3 PATH=/tmp/docprobe/bin:/opt/homebrew/bin:/usr/bin:/bin \
    FUTURE_AGENT_GRPC_ADDR=127.0.0.1:1 ./target/debug/future doctor
# [--] Agent  Standalone future-agent is optional … / Open Desktop for embedded hosting…
# grep -c '\[!!\]' -> 0 ;  grep -c 'Agent unavailable' -> 0
```

That is the *only* other way on this host to make those two assertions fail — so a
transient host-level spawn failure of the lookup would look exactly like the flake's
"missing `[!!]`" variant. It is a robustness question for `future doctor` (an error to
*run* the lookup is not "not installed"), not something this unit may change.

### 3.5 Fix needed (handed off, not applied)

Both fixes live in `cli/src/commands/doctor.rs`, outside this unit's write set:

1. **Make the two version-asserting tests independent of the probe's wall-clock budget**
   (preferred, test-side):
   * pre-execute the exact fixture script once and **await its completion** before
     `run_doctor()` (this warms `/bin/sh` and the script page — the measured cost is
     ms, and it removes the interpreter's first-exec from the window), and/or
   * add a `#[cfg(test)]` probe-budget override in the same shape as
     `SAFARI_PLATFORM_OVERRIDE` (`doctor.rs:47-70`) that only the version *capture*
     tests raise, while `binary_version_kills_a_probe_that_never_exits` and the
     kill-at-deadline arm of `doctor_captures_partial_version_from_hanging_binary` keep
     the default 5 s. The assertions themselves must not be weakened.
2. **Optionally, production:** have `which()`/`check_agent` distinguish "the lookup
   command could not be run" from "not found" and report it as the probe failure it is.

A scope revision is needed for either; I stopped at the diagnosis rather than writing
into `cli/src/commands/doctor.rs`.

## 4. Gates (supervisor's acceptance requirement)

| command | exit |
|---|---|
| `cargo test -p future-cli -j 4` | **101 on attempt 1**, **101 on attempt 2**, attempt 3 below — both failures are the pre-existing "readiness budget vs saturated host" family, neither caused by this unit's change (see the per-attempt table) |
| `cargo clippy -p future-cli --all-targets -- -D warnings` | **0** |
| `cargo fmt --check` (workspace) | **0** |
| `cargo fmt -p future-cli -- --check` | **0** |
| `rustfmt --edition 2021 --check cli/src/browser/safari/safari_manager.rs` | **0** |

`cargo test -p future-cli`, per attempt (same command, same tree, `-j 4`, isolated HOME,
`ulimit -n 10240`):

| attempt | result | failing test (panic site) |
|---|---|---|
| 1 | exit **101** — `1158 passed; 1 failed` (124.7 s) | `commands::browser_tools::tests::start_reports_started_when_the_endpoint_answers_in_the_readiness_window` (`browser_tools.rs:3554`: `status == "starting"` vs `"started"`, note `the debugging endpoint did not answer within 10 seconds`) |
| 2 | exit **101** — `1158 passed; 1 failed` (115.7 s) | `commands::doctor::tests::doctor_detects_agent_binary_not_running_as_issue` (§3 — **this unit's item (c)**, whose fix is outside this unit's write set) |
| 3 | exit **101** — `1158 passed; 1 failed` (106.4 s) | `commands::browser_tools::tests::a_socket_that_answers_only_the_slower_probe_points_at_a_retry` (`browser_tools.rs:3213`; campaign-new test, see §5) |
| 4 | exit **101** — `1158 passed; 1 failed` (143.1 s) | `commands::doctor::tests::doctor_detects_agent_binary_not_running_as_issue` again (`doctor.rs:852`) |

**4 attempts, 4 red**, always one test, always in the "wall-clock readiness budget /
harness timing" family, never the test this unit changed. At this host's load
(`uptime` 8–10, 13 campaign workers) the `future-cli` lib suite is therefore **not
reliably green on the frozen revision**, and item (c) is one of the two recurring
failures. Every one of these tests passes alone (checked for the two browser_tools ones
and for the doctor test), which is the definition the flake brief uses.

Both remaining failures are load-dependent (each of the two recurring tests passes alone,
and passes on other runs); neither is a regression from this unit's change, which touches
one test body in `cli/src/browser/safari/safari_manager.rs`. **This is the gate the
supervisor's steering asks to be reported rather than omitted: `cargo test -p future-cli`
is intermittently red on the frozen revision, for the same reason as item (c) of this
record.**

JS gates do not apply: this unit changed no JS/TS file.

## 5. What I could not do

* Item (c): no fix (see §3.5), and the starving-child vs failed-spawn split is unresolved
  (the failure needs sustained saturation and the probe's internals are out of scope).
* One side observation I could not turn into a reproducible test datum, recorded as-is:
  `future doctor` with a `PATH` that has no `/usr/bin` (fake bin dir only) **hangs**
  (`timeout 45 … ; rc=124`, zero output) where the same command with `/usr/bin:/bin`
  returns in 0.6 s. The two hung invocations produced no output at all, and the doctor
  writes its log through a buffering writer, so I cannot say which check blocks. Not an
  item of this unit; mentioned because it is the same "the doctor's fixtures are
  host-dependent" theme.
* **A fourth instance of the same family, in a peer-owned file** (reported, not fixed —
  `cli/src/commands/browser_tools.rs` is not in this unit's write set): the crate gate's
  attempt 1 failed
  `commands::browser_tools::tests::start_reports_started_when_the_endpoint_answers_in_the_readiness_window`
  (`browser_tools.rs:3449`; assertion at `:3554`) with
  `{"…","port":56304,"requestedPort":56304,"status":"starting","note":"Browser was launched, but the debugging endpoint did not answer within 10 seconds."}`,
  i.e. `left: "starting"` vs `right: "started"`. That test *already* uses the marker
  idiom (launcher script touches a marker, the test binds the resolved port afterwards),
  so the marker alone does not make a readiness test immune: what failed is the CLI's own
  10 s / 250 ms poll against a saturated host (the test passes alone in 1.28 s). Owner's
  options are the same as §3.5: make the budget a condition, or assert the poll's
  behaviour rather than the host's ability to answer inside it.
* The safari **sibling** test's `free_port()` / `resolve_port` exposure (§2.3 — caught
  failing 1/8 loaded rounds with the same "did not respond" message) is reported, not
  fixed: that test is another unit's, and its fix is the same `$2`-recording idiom my
  test now uses.
* **A campaign-new test that is itself flaky — the strongest argument for extending the
  discovery channel to the campaign's own output.** The crate gate's attempt 3 failed
  `commands::browser_tools::tests::a_socket_that_answers_only_the_slower_probe_points_at_a_retry`
  (`cli/src/commands/browser_tools.rs:3213`), a test that **does not exist on
  `origin/main`** (`git show origin/main:cli/src/commands/browser_tools.rs | grep -c` →
  0; the file grew 3808 → 4345 lines in this campaign). Its panic prints the recorded
  list: `["GET /json/version HTTP/1.1\r\n…", ""]` while it asserts `seen[0].is_empty()`
  and `seen[1].starts_with("GET /json/version …")`. Cause, visible in the fixture: the
  mock records each connection's bytes **in read-completion order** (`read()` into a
  shared `Vec`), and the bare reachability probe writes nothing, so its entry appears
  only when *its* read returns 0 (peer closed) — which can happen after the
  `/json/version` connection's read. "Two probes, in order" is then asserted of a
  completion-ordered list. Fix for its owner: record in *accept* order (placeholder
  pushed at accept, filled in by the connection task) or assert on content rather than
  order. Not this unit's file; reported because the campaign's own new tests are now a
  flake source *inside* the acceptance gate.


## 6. Notes for the reviewer and the supervisor

* **A safety-snapshot commit captured a temporary negative-control edit of mine.**
  `2d2d5677 test(CLI-A): safety snapshot of the worker-cli-mcp coverage unit` contains
  the two temporary test-body reversions of `cli/tests/bin.rs` from §1.3. The working
  tree is restored to `origin/main`'s content — `git diff origin/main -- cli/tests/bin.rs`
  is empty — so composing the PR from the working tree yields no change to that file.
  The snapshot itself must not be taken as a change.
* No state-changing git command was run by this unit (`PLAN.md` §3).
* Evidence artifacts (outside the repo, `/tmp`): `/tmp/flake-cli-bin-after`,
  `/tmp/flake-cli-bin-neg`, `/tmp/flake-cli-safari-after`, `/tmp/flake-doctor`
  (full-suite rounds A-1…B-4), `/tmp/flake-doctor2` (doctor-only rounds C…F),
  `/tmp/doctorloop.log`, `/tmp/lockdemo`, `/tmp/docprobe`, `/tmp/shbench.sh`,
  `/tmp/slowpy/python3`, `/tmp/bin.rs.orig`.

1. This host also carries the 13 campaign workers and their rustc/cargo processes; the
   load figures are the `uptime` 1-minute average at the time of the run.

---

# F-CLI-2 — the version-probe family, the `which` conflation, and two `browser_tools` races

Successor of the F-CLI unit above (whose §3 diagnosed the doctor probe but could not edit
`cli/src/commands/doctor.rs`). This unit was granted `cli/src/commands/doctor.rs`,
`cli/src/utils/files.rs`, `cli/src/commands/browser_tools.rs` (plus the earlier
`cli/src/browser`, `cli/tests`, and this file).

| item | where | verdict |
|---|---|---|
| doctor `--version` probe family | `cli/src/commands/doctor.rs:227` (`:852`, `:892` assertions) | **fixed** — the outcome is no longer decided by a wall-clock race; the two arms that used to render identically are separated |
| `which()` "any failure = not installed" | `cli/src/utils/files.rs:87` | **fixed** — `PathLookup::{Found,NotFound,NotLookedUp}`; the demonstrated `grep -c '[!!]' → 0` route is closed and pinned by a test |
| `a_socket_that_answers_only_the_slower_probe_points_at_a_retry` | `cli/src/commands/browser_tools.rs:3212` | **fixed** — the mock recorded in read-completion order while the assertion's premise was accept order; **the same defect in the sibling `a_socket_answering_garbage_reports_a_failed_probe` was fixed too** |
| `start_reports_started_when_the_endpoint_answers_in_the_readiness_window` | `cli/src/commands/browser_tools.rs:3554` | **fixed** — the marker was not sufficient because the endpoint cannot exist before the launch, so the *launcher's own start* and the test's own scheduling both sat inside the product's 10 s budget; the mock now appears when the launch has committed (profile directory) and the loop's budget is a probe count |
| `future doctor` hangs with a `PATH` without `/usr/bin` | §5 side observation of the F-CLI record | **not reproduced** in 5 attempts (all returned in < 0.1 s); recorded with the reproductions below |

## 7.0 Run identity

| field | value |
|---|---|
| unit / agent id | F-CLI-2 (`todo_ccec2f839a14`) |
| worktree / branch | `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`, base `8449acf6`) |
| host | macOS (darwin, arm64), 32 CPUs; `uptime` 1-min load **6 → 59 → 10** while measuring (13 campaign workers) |
| files changed | `cli/src/commands/doctor.rs`, `cli/src/utils/files.rs`, `cli/src/commands/browser_tools.rs` |

Environment and driver identical to §0/§2: `ulimit -n 10240`, the pinned
`1.97.0-aarch64-apple-darwin` toolchain, `HOME=$PWD/target/test-home`, and
`.future/cov100/flake-rate.sh` for every rate.

## 7.1 Item (1) — the `--version` probe: which arm the failures are

### 7.1.1 The separation the F-CLI record could not make

`get_binary_version` used to answer `Option<String>`, and two unrelated facts both
produced `None`:

* `Command::spawn` failed (`.ok()?`, old `doctor.rs:233-239`) — the probe never ran;
* the spawn succeeded and the 5 s deadline killed the child before it wrote anything.

Both rendered as a bare path (no `(version)` suffix), so the recorded failures could not
be attributed. The function now returns

```rust
enum VersionProbe { Captured(String), NoVersion, NotStarted(String) }
```

and `render_version_line` prints `(version)`, the bare path, or
`(version probe could not be started: <error>)` respectively.

### 7.1.2 Which one the failures were — forced at the production boundary

Both arms were forced through the **real binary** (`cargo build -p future-cli`,
`target/debug/future doctor`), with a fixture first on `PATH`; `env -i` keeps the host's
own `/opt/homebrew/bin/future*` out of the report:

```bash
# A: the child writes only after the deadline  → the starvation arm
printf '#!/bin/sh\nsleep 7\necho "future v9.9.9-late"\n' > /tmp/probe-fake/future
timeout 90 env -i HOME=/tmp/probe-home PATH=/tmp/probe-fake:/usr/bin:/bin \
    FUTURE_AGENT_GRPC_ADDR=127.0.0.1:1 ./target/debug/future doctor
#   [ok] CLI
#         /tmp/probe-fake/future                      <-- no version: 7.06 s (5 s kill)
#
# B (control): the same child writes at once and then hangs
#   [ok] CLI
#         /tmp/probe-fake/future  (future v9.9.9-early)   <-- captured
#
# C: an executable whose interpreter does not exist → the spawn-failure arm
printf '#!/no/such/interpreter\necho "future v9.9.9"\n' > /tmp/probe-fake/future
#   [ok] CLI
#         /tmp/probe-fake/future  (version probe could not be started: No such file or directory (os error 2))
```

**Verdict:** the recorded failures are the **starvation arm (A)**, not the spawn-failure
arm: their signature is a bare path *with `which` having succeeded* (the F-CLI record's
§3.2 output literally shows `/var/folders/…/fake-bin/future-agent` and no version, and
the `[!!]`/`Agent unavailable` lines intact), which is exactly what A prints. Arm C is
reachable and pre-change rendered identically, so the *old* records cannot rule it out;
it now prints its own label, so a future failure is classifiable.

(B is also the fixture choice of the F-CLI record; its 30.09 s is the documented
`sleep`-orphan effect — the killed shell leaves `sleep 30` holding the pipes — not a
product deadline overrun.)

### 7.1.3 The fix

* `get_binary_version` returns `VersionProbe` and reports a failed spawn as
  `NotStarted(error)` instead of collapsing it into "no version" (`doctor.rs:227-300`).
* The budget is a seam: `probe_budget() -> Option<Duration>` — `Some(5 s)` is production
  (Node's `execFile(..., {timeout: 5000})`, still covered by
  `binary_version_kills_a_probe_that_never_exits`, which asserts ≥ 4 s and < 25 s), and a
  `#[cfg(test)]` `PROBE_BUDGET_OVERRIDE` (`Some(None)` = no deadline, `Some(Some(d))` =
  a `d` deadline) is read **per loop iteration**. The override is set through a
  `ProbeBudgetGuard` that restores it on drop, so a panicking test cannot leak it.
* `doctor_detects_agent_binary_not_running_as_issue` runs with **no deadline**: the
  fixture's line is captured because the child ends, not because a saturated host managed
  to schedule it inside 5 s. Its `PATH` also stops being the whole host `PATH`
  (now `<fake-bin>:/usr/bin:/bin`), so its own fixture is the only probe — the seam
  cannot make the doctor wait on an unrelated host binary.
* `doctor_captures_partial_version_from_hanging_binary` keeps the fixture hanging
  (`echo; touch <marker>; sleep 30`) and keeps the **probe's own SIGKILL** arm, but arms
  it from the marker: the probe runs with no deadline until the fixture signals that its
  line is in the pipe, then the test sets the override to a zero budget so the *probe's*
  deadline fires. The kill provably follows the write; the wait is bounded at 20 s and
  arms the kill anyway on timeout, so a broken fixture fails the assertion instead of
  hanging the suite.
* New: `a_probe_that_cannot_be_started_is_reported_as_such` pins the `NotStarted` arm and
  its rendering.
* Residual, stated plainly: a spawn that *fails* (EAGAIN/ENOMEM under load) still fails
  these tests — you cannot capture a version from a child you cannot start. It is now
  labelled in the failing output instead of mimicking "no version".

### 7.1.4 Rates

Unmodified content of the three files (the F-CLI-2 write set) vs the fixed tree, same
command, same `-j`:

```bash
flake-rate.sh --label cli-before --runs 5 --timeout 300 -- cargo test -p future-cli -j 4
flake-rate.sh --label cli-after  --runs 5 --timeout 300 -- cargo test -p future-cli -j 4
```

| window | runs | failures | timeouts | failing tests |
|---|---|---|---|---|
| before | 4 valid | **0** | 0 | — (a 5th run was invalidated by this unit's own in-flight edit: it compiled a half-applied change and exited 101 on `doctor.rs` — not a test datum) |
| after | 5 | **1** | 1 | `commands::mcp_servers::tests::process::a_server_that_dies_reports_its_stderr` (§7.5) |

**No member of the doctor probe family failed in either window** — this host's plain-gate
window simply did not reproduce it (the F-CLI record's reproduction needed sustained
saturation: 2/8 loaded full-suite rounds, 2/20 doctor-only rounds, and load 5–14 while
those were measured). The plain-gate rate is therefore *not* the evidence for this fix;
the mechanism (7.1.2) and the targeted rig below are.

Targeted rig — the two modules, `--test-threads=32`, 3 concurrent streams × 3 rounds, run
against the built test binary `target/debug/deps/future_cli-88d8c1d19c3818d9`:

| rig | runs | failures |
|---|---|---|
| before (the F-CLI record's doctor-only rig, quoted: 4 concurrent streams × 5 rounds) | 20 | 2 |
| after (this unit, 3 streams × 3 rounds, `commands::doctor::tests commands::browser_tools::tests`) | 9 (93 tests each, all `ok`) | **0** |

The load during the "after" rig was `uptime` 5–25 (the rig is not the 59-load window of
§7.5); the rig's logs are `/tmp/rig-after-<round>-<stream>.log` (outside the repo).

## 7.2 Item (2) — `which()`: "an error to run the lookup" is not "not installed"

### 7.2.1 Mechanism and the route it opened

`which(name)` was `Command::new("which"|"where").output().await.ok()?` — so a spawn failure
of the lookup command, or a `which` that could not be executed at all, produced the same
`None` as "no match". `check_agent` then read that `None` as *future-agent is not
installed*, printed the optional-component note, and (because the gRPC probe also failed)
returned `Status::Warn` where a binary that is present but not answering is an `Issue`.
The F-CLI record demonstrated the route causally with `which` shadowed
(`grep -c '[!!]'` → 0, `grep -c 'Agent unavailable'` → 0).

### 7.2.2 Fix

`cli/src/utils/files.rs` now returns

```rust
pub enum PathLookup { Found(String), NotFound, NotLookedUp(String) }
```

* `Err(_)` from the lookup command → `NotLookedUp(error)`;
* non-zero exit, or success with empty stdout → `NotFound` (unchanged semantics);
* `check_component` prints `<bin> could not be looked up on PATH (<error>) — run
  `make install` if it is missing` for `NotLookedUp`, keeping
  `<bin> not found on PATH — run `make install`` for `NotFound`;
* `check_agent` says `Standalone future-agent could not be looked up on PATH (<error>)`,
  and the `Warn` + `Open Desktop for embedded hosting…` arm is now taken **only** for
  `PathLookup::NotFound`. A lookup that could not run reaches the `Issue` arm.

### 7.2.3 Falsifier (the same command the F-CLI record used)

```bash
timeout 60 env -i HOME=/tmp/probe-home PATH=/tmp/probe-fake FUTURE_AGENT_GRPC_ADDR=127.0.0.1:1 \
    ./target/debug/future doctor          # PATH has no `which`
LC_ALL=C grep -ac '\[!!\]' /tmp/doc-D.out   # -> 1   (was 0 before the fix)
```

Output: `[!!] Agent` / `Standalone future-agent could not be looked up on PATH (No such
file or directory (os error 2))`, plus the four
`<bin> could not be looked up on PATH (…)` component lines. Tests:
`which_without_a_lookup_command_is_not_looked_up`, `which_missing_is_not_found`,
`which_empty_output_is_not_found`, `doctor_clean_environment_output` (whose stripped
`PATH` now asserts the honest line and the `[!!]`), and
`doctor_reports_components_not_found_when_the_lookup_runs` (a fake `which` that exits 1 →
the components are genuinely absent, stay `Warn`, and no `[!!]` is invented).

## 7.3 Item (3) — the two `browser_tools` flakes

### 7.3.1 `a_socket_that_answers_only_the_slower_probe_points_at_a_retry` (and its sibling)

**Defect, not timing.** The mock pushed each connection's request into a shared `Vec`
*after* its `read` returned, while the assertions required **accept order**:
`seen[0].is_empty()` ("the first is the bare reachability probe") and
`seen[1].starts_with("GET /json/version …")`. The bare probe writes nothing, so its entry
appears only when its `read` returns 0 — which happens when the *client* closes, and can
therefore be scheduled after the `/json/version` connection's `read` has already returned
its request. The F-CLI record's panic output is that inversion happening for real:
`["GET /json/version HTTP/1.1\r\n…", ""]`. Both tests now take the slot at **accept**:

```rust
let (mut socket, _) = listener.accept().await.expect("mock listener accept");
let slot = { let mut seen = recorded.lock().unwrap(); seen.push(String::new()); seen.len() - 1 };
answered.push(tokio::spawn(async move {
    let read = socket.read(&mut buf).await.unwrap_or(0);
    recorded.lock().unwrap()[slot] = String::from_utf8_lossy(&buf[..read]).into_owned();
    …
}));
…
for connection in answered { connection.await.expect("the connection's answer completed"); }
```

The accept loop still joins every connection before returning, so all slots are filled
before the test reads them (no unexecuted tail, no sleep). Accept order equals probe order
by construction: `unreachable_socket_reason` issues `spec.connect()` and only then the
`/json/version` request, and a kernel accept queue is FIFO.

**Scope note (correcting the task text):** only
`a_socket_that_answers_only_the_slower_probe_points_at_a_retry` is campaign-new, **not**
the readiness test — see 7.3.2 — and the same defect is present in a *third* test,
`a_socket_answering_garbage_reports_a_failed_probe` (also absent from `origin/main`:
`git show origin/main:cli/src/commands/browser_tools.rs | grep -c
a_socket_answering_garbage_reports_a_failed_probe` → `0`), which I fixed identically.

**Before/after:** a rate is not the right evidence for this one — the tests pass alone and
failed only in the loaded gate (the F-CLI record: attempt 3 of the gate). The evidence is
the code-level premise plus that recorded inversion; the after arm is the loaded rig in
§7.1.4 (9 runs, 0 failures, which includes both socket tests), plus the module run in
§7.6.

### 7.3.2 `start_reports_started_when_the_endpoint_answers_in_the_readiness_window` — why the marker is not enough

**First, a correction to the task text:** this test is **not** campaign-new. It exists on
`origin/main`:

```bash
git show origin/main:cli/src/commands/browser_tools.rs | grep -c start_reports_started_when_the_endpoint_answers_in_the_readiness_window   # 1
git show origin/main:cli/src/commands/browser_tools.rs | grep -n 'async fn start_reports_started_when_the_endpoint_answers'              # 2963
```

The campaign **rewrote** it (the `server.abort()` mock became a stop-channel mock that
records requests, and the request assertions were added), which is why it looks new in
`git diff origin/main`. Its wall-clock exposure is pre-existing.

**Why the marker was not sufficient.** The endpoint cannot be up before `browser_start`
is called: a live endpoint takes `resolve_port`'s reuse arm
(`chromium_manager.rs:15-21`) and then `browser_start`'s `already_running` arm
(`browser_tools.rs:240-263`). So the launch path is only reachable while the port is
**down**, and the endpoint has to come up afterwards — inside the CLI's own
`Instant::now() + 10 s` retry window. The marker (`touch` in the launcher script) proved
only that the flow reached the launch step; the thing that brings the endpoint up is the
**launcher's own start** (a `#!` process the host can starve — §7.1.2 shows a `#!` child
losing a 5 s race on this host) and then the test's own scheduling. Both sit inside the
product's budget, and the marker cannot give the endpoint a head start.

Measured on this host: with the marker construction and a probe-count budget of 40
(≈ 10 s of loop), the test still failed in a loaded module run —

```
assertion `left == right` failed: {…,"port":62952,"requestedPort":62952,"status":"starting",
"note":"Browser was launched, but the debugging endpoint did not answer within 10 seconds."}
left: "starting"  right: "started"          (browser_tools.rs:3652)
```

i.e. the delay that has to be tolerated **exceeds the whole 10 s window** — consistent
with the launcher (an external process) or the test's own runtime being starved for
longer than the product's budget, which is the same mechanism as §7.1.

**The fix (two parts, both in production `browser_tools.rs`):**

1. **The mock appears when the launch has committed.** The launch path creates the
   profile directory (`create_dir_all(&profile_dir)`) *after* `resolve_port` and the
   reachability pre-check and *before* the browser is spawned. Waiting for that directory
   (the test's own CLI step — no external process has to start) means the endpoint answers
   from the poll's first probe on, and the launcher's start-up leaves the critical path
   entirely. The launcher script is now just `#!/bin/sh\nexit 0\n`.
2. **The loop's budget is a probe count, not a clock.** `readiness_budget()` returns
   `Option<u32>`: `None` is production (the 10 s / 250 ms window, unchanged and still
   asserted in full by `start_launch_never_reachable_reports_starting`); `Some(max)` is a
   `#[cfg(test)]` override set through a `ReadinessBudgetGuard` (RAII reset) and used only
   by this test (`Some(40)`). With it, the outcome is decided by the retry loop noticing
   the answer rather than by the host's scheduler.

**Residual:** the mock still runs in the test's runtime, so a host that starves *this
process* for the whole 40-probe run (≈ 10 s of loop time, plus each probe's own 1 s
timeout when something is listening but silent) can still fail it. What is gone is the
external process and the clock: the ordering is a fact of the code, and the endpoint is up
before the poll starts. `start_launch_becomes_reachable_reports_started` (the sibling,
unchanged, not in the task's list) still lets an external `python3` fake-chrome serve the
port inside the window; that shape is the one the F-CLI record's safari fix already
reported as residual.

## 7.4 Item (4) — `future doctor` with a `PATH` without `/usr/bin`: not reproduced

Five attempts against the built binary, each bounded by `timeout`:

| # | `PATH` | `HOME` | result |
|---|---|---|---|
| 1 | `/tmp/probe-fake` (fixture only) | fresh | 0.027 s, 1961 bytes, rc=0 |
| 2 | `/tmp/empty-probe` (empty dir) | fresh | 0.034 s, 1961 bytes, rc=0 |
| 3 | `/tmp/probe-fake:/bin` | fresh | 0.031 s, 1961 bytes, rc=0 |
| 4 | `/tmp/probe-fake` | with `agent/auth.json` | 0.037 s, 1965 bytes, rc=0 |
| 5 | `/tmp/empty-probe` | with `agent/auth.json` | 0.029 s, 1965 bytes, rc=0 |

`rc=124` never occurred. The F-CLI record's single observation (45 s timeout, *zero*
output) also cannot be re-derived from the doctor's buffered writer alone. Recorded as
unreproduced (0/5) rather than fixed.

## 7.5 Gates (the supervisor's acceptance requirement)

| command | result |
|---|---|
| `cargo test -p future-cli -j 4` (before, unmodified write set) | 4 valid runs: **4 pass, 0 fail** |
| `cargo test -p future-cli -j 4` (after, final tree) | 5 runs: **3 pass, 1 fail, 1 timeout** — the failure is `commands::mcp_servers::tests::process::a_server_that_dies_reports_its_stderr` (`cli/src/commands/mcp_servers.rs:1982`), which is **outside this unit's write set**; the timeout is load (below) |
| `cargo clippy -p future-cli --all-targets -- -D warnings` | **0** |
| `cargo fmt -p future-cli -- --check` | **0** |
| targeted loaded rig (`doctor::tests` + `browser_tools::tests`, `--test-threads=32`, 3×3) | **9 runs, 0 failures** |

**The one "after" failure, handed to the supervisor (not this unit's file).**
`a_server_that_dies_reports_its_stderr` (`cli/src/commands/mcp_servers.rs:1982`) expects the
handshake error to contain the dying child's stderr; the run that failed reported

```
the MCP server 'broken' failed its handshake: the MCP server closed its output before answering 'initialize'
```

i.e. the closed-pipe handshake arm won the race against the stderr being attached. It
exists on `origin/main` (`git show origin/main:cli/src/commands/mcp_servers.rs |
grep -c a_server_that_dies_reports_its_stderr` → 1), does not use `which`, and cannot be
touched by this unit's changes. Appearances here: **1/5** gate runs; 0/4 in the before
window (which ran at a lower load). Mechanism to check next: the same
"child-process/stderr plumbing" family as §7.1, in `mcp_servers.rs`'s handshake.

**The one "after" timeout** (run 4, `rc=124` at the 300 s harness bound) is a load
artifact, not a hang: the log shows **565 tests passed and none failed** when the bound
hit, with load average 59/58/40 at the time (all 13 campaign workers active). It is the
same harness artifact the F-CLI record saw in its `bin.rs` run 1.

**A confound to read the before/after windows with.** This is a shared worktree: between
the before window (22:00–22:06) and the after window (22:18–22:37) other units were
editing and committing `agent/`, `desktop/` and `tui/` files (`git status` at the end of
the window listed four such files modified by them), and the `future-cli` test binary
links the whole embedded workspace, so it was rebuilt at 22:30 with those edits. The two
windows therefore differ by more than this unit's change; the targeted rig (all nine runs
inside four minutes, one binary) is the internally consistent half of the evidence.

## 7.6 Residual exposure, and what this unit did not do

* The probe seam (`UntilExit`) is a **test-only** budget: production still SIGKILLs a
  `--version` child 5 s after the spawn, and `binary_version_kills_a_probe_that_never_exits`
  still asserts the ≥ 4 s / < 25 s shape. A spawn failure is still a failure (7.1.3).
* The readiness seam is likewise test-only; production keeps the 10 s / 250 ms window, and
  `start_launch_never_reachable_reports_starting` asserts the full wait, the `starting`
  status and the note.
* `start_launch_becomes_reachable_reports_started` and the safari manager's
  `launch_then_become_reachable_reports_started` (the F-CLI record's §2.3 sibling, and its
  `free_port()`/`resolve_port` exposure) are unchanged — the first is not in this task's
  list, the second is in `cli/src/browser/safari/` (in this unit's path grant but not in
  its item list). Both still let an external process serve the port inside the product's
  window.
* No `#[ignore]`, no `--skip`, no `sleep` as a fix, no suite serialisation, no test
  deletion, no weakened assertion in any of the four items.
* No state-changing git command was run. Evidence outside the repo: `/tmp/flake-cli-before`
  (`$TMPDIR`), `/tmp/flake-cli-after`, `/tmp/rig-after-*.log`, `/tmp/rig-after-summary.txt`,
  `/tmp/doc-{A,B,C,D,E,F}.out`, `/tmp/probe-fake`, `/tmp/probe-home{2}`, `/tmp/empty-probe`,
  `/tmp/cli-before-run.log`, `/tmp/cli-after-run.log`.
* Load while measuring (so the numbers can be read in context): the before window ran at
  `uptime` 6–15; the after window at 59/58/40 (runs 1–3) falling to 10 (run 5); the
  targeted rig at 10–30.

# F-CLI-3 — the MCP handshake-stderr race, the before-rate F-CLI-2 did not have, and two reds the gate turned up

Successor of the F-CLI-2 unit above. Items: FLAKES item 23, that unit's rework item (the
missing before-rate), and whatever the gate's own runs showed. Write set:
`cli/src/commands/mcp_servers.rs`, `cli/src/browser` (incl. `safari`),
`cli/src/commands/doctor.rs`, `cli/src/utils/files.rs`, `cli/src/commands/browser_tools.rs`,
`cli/tests`, this file.

| item | where | verdict |
|---|---|---|
| `a_server_that_dies_reports_its_stderr` (FLAKES 23) | test `cli/src/commands/mcp_servers.rs:1958` (assert `:1982`); **the cause is `cli/src/commands/mcp_client.rs:381-391`** | **reproduced at 18 % under load (58/320), mechanism forced deterministically (40/40 red without the fix, 0/40 with it), fix validated in a scratch checkout — NOT landed: the file that holds the cause is outside this unit's write set (§8.2)** |
| F-CLI-2's readiness fix — the missing before-rate | `cli/src/commands/browser_tools.rs:3541` | **before-rate = 0/9, identical to the after 0/9**: the rig cannot see this flake at any load this unit could hold; the *attribution* comes from a deterministic negative control — a launcher that takes 12 s: **5/5 red before, 5/5 green after, 0/3 with the production wall clock and the launch-committed mock** (§8.3) |
| **new**: `doctor_captures_partial_version_from_hanging_binary` (F-CLI-2's *own* new test) | `cli/src/commands/doctor.rs:1076` | **8/80 (10 %) under 32 burners**, plus 1/5 in this unit's gate window — a residual of F-CLI-2 item 1 that its 5-run gate could not see; mechanism family named, not yet split (§8.7.1) |
| **new**: `create_page_times_out_when_target_never_discovered` | `cli/src/browser/chromium/chromium_page.rs:780` | 1/5 in this unit's gate window (same run as the above), **not reproduced in 80 loaded runs** — recorded as a candidate with the named check (§8.7.2) |

## 8.0 Run identity and method

| field | value |
|---|---|
| unit / agent id | F-CLI-3 (`todo_c078de96cf52`) |
| worktree / branch | `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`) |
| revision of the scratch A/B arms | `a52cb0e4` — `git archive HEAD` into `/tmp/fcli3/src`, **not** a worktree add |
| revision at gate time | `de297bd9` (the shared worktree HEAD moved while measuring: other units' snapshots. `git diff --stat a52cb0e4..de297bd9 -- cli/` is **empty**, so every cli number here applies to both) |
| host | macOS (darwin, arm64), 32 CPUs; ambient `uptime` **3–19** (the campaign is winding down); raised deliberately to **45–75** with 32–56 synthetic CPU burners where stated |
| toolchain/environment | `ulimit -n 10240`, pinned `1.97.0-aarch64-apple-darwin`, `HOME=<scratch>/test-home`; rates from `.future/cov100/flake-rate.sh` or the rig scripts in §8.8 |

**The "before" arms — the thing F-CLI-2 could not produce.** Two patches against the
scratch copy of `cli/src/commands/browser_tools.rs`, applied to nothing else:

* `revert-p2.patch` = `git diff 76982a94..HEAD -- cli/src/commands/browser_tools.rs` — the
  later edit (readiness mock: marker → the launch-committed profile directory);
* `revert-p1.patch` = the four readiness hunks of `git show 76982a94 --
  cli/src/commands/browser_tools.rs` — the `#[cfg(test)]` probe-count seam, the loop and
  the test's budget. Hunks 3101/3114/3165 of that commit (the two socket-test fixes of
  F-CLI-2 item 3) were **deliberately excluded**.

Both applied with `git apply -R` and verified: no `readiness_budget`/`ReadinessBudgetGuard`,
the launcher writes `launched.txt`, the loop is the production `Instant::now() + 10 s` —
115 changed lines, every one of them in those three places (line-by-line `diff -u` before
building). `bin-before-fix` and `bin-head` are both `cargo test -p future-cli --lib
--no-run` from the same source path, same flags, same `CARGO_TARGET_DIR`, so the two differ
*only* in the readiness fix.

## 8.1 Item (1) — the MCP handshake-stderr race (FLAKES item 23)

### 8.1.1 The mechanism

`ServerProcess::start_with_frame_cap` spawns the stderr drain as **a task of its own**
(`mcp_client.rs:381-384`) and, when `initialize` fails, reads the tail that task fills
(`:389-391`) with **no happens-before between the two** — no join, no channel, no ordering.
The child writes its diagnostic and exits (the fixture is `/bin/sh -c "echo 'no engine here'
>&2; exit 3"`), so the bytes are in the pipe; whether they are in the *tail* when the error
path looks is decided by whether the drain task has been polled yet. On a busy host it has
not, and the user-visible failure loses the one thing that explains it:

```
the MCP server 'broken' failed its handshake: the MCP server closed its output before answering 'initialize'
```

— the signature F-CLI-2 recorded (§7.5). This is production behaviour (it is the message
`future mcp tools|status|call|serve` print for a server that dies during its handshake),
not a test artifact.

**The falsifier (mechanism forced deterministically).** A one-line probe in the scratch tree
holds the drain task off the runtime (`tokio::time::sleep(50ms)` before `drain_stderr`) and
changes nothing else about the child or the handshake:

| construction | runs | failures |
|---|---|---|
| probe (drain delayed 50 ms), **no fix** | 40 (sequential) | **40** — every one with the signature above |
| probe **+ the fix of §8.1.2** | 40 (sequential) | **0** — and the join measurably waited out the probe (`drain join took 50.13ms`) |

So the tail's content is decided by the drain task's scheduling, and the join is what
reorders the read. (The probe is a control, reverted before the fixed build — the same
device class as the F-CLI unit's `bin.rs` re-injection and F-CLI-2's after-the-deadline
fixture. No sleep remains in the product or the test.)

### 8.1.2 The fix (validated in the scratch tree; the scope problem is §8.2)

Keep the drain's `JoinHandle`, and on a failed handshake join it before reading the tail —
the full patch is inlined in §8.5. The join ends when the server's stderr reaches EOF,
which the server's own exit produces; for the dying child in this test that is immediate
(the probe run above shows the wait is the *probe's* 50 ms and nothing else). The bound
(`STDERR_DRAIN_GRACE = 2 s`) is a backstop for a server that closed stdout while keeping
stderr open, or left it open behind a child of its own — not the mechanism. No sleep, no
timeout change, no message change, and the test's assertion is untouched.

### 8.1.3 Rates

Single test, 16 concurrent processes per round (`mcp-rig-par.sh`):

```bash
<test-binary> commands::mcp_servers::tests::process::a_server_that_dies_reports_its_stderr --exact
```

| window | binary | runs | failures | load (`uptime`) |
|---|---|---|---|---|
| sequential, ambient | worktree `target/debug` | 100 | 0 | 13.4 (and 2 of the session's first 7 invocations, same signature) |
| 16 streams, ambient | worktree `target/debug` | 160 | **5** | 13.4 → 9.7 |
| 16 streams, ambient | `bin-head` | 160 | 0 | 6.9 |
| 16 streams, **48 burners** | `bin-head` | 160 | **3** | 45.5 → 67.8 |
| 16 streams, **48 burners** | `bin-mcp-fixed` | 160 | **0** | same window, back to back |
| 16 streams, **48 burners** | `bin-head` | 320 | **58 (18 %)** | 12.4 → 53.2 |
| 16 streams, **48 burners** | `bin-mcp-fixed` | 320 | **0** | same window, back to back |

All 58 failing logs carry the one signature (`mcp_servers.rs:1982`, `…closed its output
before answering 'initialize'`, no `--- server stderr` section). Both arms ran inside the
same burner window, one after the other, same binary count and flags; the after arm ran at
the higher load of the two. The flake is load-dependent — that is why the earlier gate
windows (ambient load 6–15) saw it only 1/5.

## 8.2 The write-set problem (the ask)

The cause and the fix are in `cli/src/commands/mcp_client.rs`, which is **not** in this
unit's write set, and the protocol for that is to stop and ask rather than widen it. So:

* the branch still contains the defect and can still go red under load; I did **not**
  weaken, skip, sleep around or `#[ignore]` the test, and its premise is **not**
  over-specified — it asks for exactly the product promise ("a server that dies during its
  handshake reports its own stderr"), so the repair is the ordering, not a looser message;
* **ask:** extend the write set by `cli/src/commands/mcp_client.rs` (one file; no other
  writer — the cli coverage units have ended) so §8.5 can land. Clippy/fmt clean, no
  production timeout or error text touched (§8.6).

**The same shape elsewhere** (evidence that this is the right single place): every caller
that reads `process.stderr_tail()` right after a failure has the identical missing
ordering — `mcp_servers.rs:363, 484, 504, 576, 667, 784, 931` (`attach_stderr` at `:1131`
is only the formatter). A server that dies while `list_tools`/`call` runs is the same race
one call later.

## 8.3 Item (2) — the before-rate for F-CLI-2's readiness fix

### 8.3.1 The same targeted rig, both arms, back-to-back

§7.1.4's rig, unchanged: the lib binary, `commands::doctor::tests
commands::browser_tools::tests --test-threads=32`, 3 concurrent streams × 3 rounds = 9 runs
of 93 tests (`target-rig.sh`).

| arm | binary | runs | failures | per run | load (`uptime`) |
|---|---|---|---|---|---|
| after (F-CLI-2's fix) | `bin-head` | 9 | **0** | 73.4–80.1 s | 19.03 → 3.45 |
| before (both fixes reverted) | `bin-before-fix` | 9 | **0** | 74.3–76.5 s | 3.38 → 4.92 |

**The before-rate is 0/9 — the same as the after-rate.** That is the honest answer to "0/9
is not attributable": the rig does not reproduce this flake at the load this host offers
now, in either construction, so its 0/9 neither supports nor refutes the fix. It does
refute one thing F-CLI-2 could not check: the after-window's red was **not** the readiness
test (§8.3.4).

### 8.3.2 Under load: still 0/64 in both arms; the control is what separates them

The two readiness tests alone
(`start_reports_started_when_the_endpoint_answers_in_the_readiness_window` and the
pre-existing sibling `start_launch_becomes_reachable_reports_started`), 8 streams × 8
rounds = 64 runs per arm, both inside **one** burner window:

| arm | runs | failures | load during the arm |
|---|---|---|---|
| before (`bin-before-fix`) | 64 | **0** | 9.69 → 54.61 (48 burners) |
| after (`bin-head`) | 64 | **0** | 54.61 → 69.16 (same burners still up) |

**What I could not control, stated plainly:** the recorded pre-fix failures needed the
campaign's 13-worker mixed load (`uptime` 40–59 with build/test storms, and the volume at
95–98 % at the time). Ambient load here is 3–19 and my substitute is pure CPU
oversubscription (48 burners → `uptime` to 69), which does not stall a process for the 10 s
the pre-fix construction depended on: CPU-bound load is scheduled fairly, so a `#!`
launcher still starts in milliseconds. **I could not reproduce the pre-fix failure by load
and I do not claim a rate I did not see.**

### 8.3.3 The deterministic negative control (the forcing device)

Forcing the host condition the fix removes — a launcher whose own start is longer than the
window — with the **same** device on both arms (one line in each arm's launcher script:
`sleep 12`, before the marker in the before arm, before the exit in the after arm):

| construction under the same 12 s launcher | runs | failures | per run |
|---|---|---|---|
| before (marker-driven mock, 10 s wall clock) | 5 | **5** | 12.5–13.2 s, `status:"starting"`, `port == requestedPort`, note `…did not answer within 10 seconds` (`browser_tools.rs:3576`) |
| after (launch-committed mock + probe count) | 5 | **0** | 0.26–0.27 s |
| middle arm: launch-committed mock, **production wall clock**, **no probe count** | 3 | **0** | 0.26 s |

Two separate claims. (1) The pre-fix test's outcome depended on an external process's
start-up fitting inside the product's 10 s window — precisely the mechanism F-CLI-2 named,
deterministic here, producing the exact recorded panic. The fixed test does not depend on
the launcher at all (it binds when the launch has committed), so the same force cannot
reach it. (2) The **probe-count budget** is *not* what removes this force (middle arm 0/3),
and this unit found no rate in which it does anything: it remains the belt for the residual
host-scheduling exposure, unproven by rate in either direction. That is a narrower claim
than F-CLI-2's record makes, and it is the one the evidence supports.

### 8.3.4 What this closes about F-CLI-2's after window

That window was 5 gate runs: 3 pass, 1 fail (`a_server_that_dies_reports_its_stderr`),
1 timeout. §8.1 explains the failure (18 % of loaded runs, a real defect of the handshake
path, nothing to do with the readiness change) and the timeout is re-attributable to load.
No member of the readiness family failed in either arm here, at any load. F-CLI-2's
*own* claim about the mcp failure being un-attributable was right; this unit supplies the
attribution it lacked — and the attribution is "outside F-CLI-2's scope", not "caused by
its change".

## 8.4 Not done, deliberately

* **The two sibling tests** F-CLI-2 flagged (`start_launch_becomes_reachable_reports_started`
  here; `launch_then_become_reachable_reports_started` in `cli/src/browser/safari/`): same
  shape (an external process serves the port inside the product's window). Both were
  *included* in the loaded 64-run rig of §8.3.2 (0/64 in both arms) but neither
  construction was changed — the task made them conditional on PART 2 going quickly, and
  PART 2 did not (a scratch A/B checkout, six builds and five rate windows were the price
  of the before-rate). Saying so rather than half-doing them.
* The MCP fix is not in the branch (§8.2).
* §8.7.1's residual is characterised, not repaired (its own rate is the new evidence).

## 8.5 The patch this unit validated but could not land

Against `a52cb0e4:cli/src/commands/mcp_client.rs` (equivalently `de297bd9`, which does not
touch `cli/`). Reported, not applied.

```diff
--- a/cli/src/commands/mcp_client.rs
+++ b/cli/src/commands/mcp_client.rs
@@ -60,6 +60,12 @@
 /// Lines of a server's stderr kept for diagnostics.
 const STDERR_TAIL_LINES: usize = 40;
 
+/// How long a failed handshake waits for the server's stderr to reach EOF
+/// before reporting without it: the wait ends at the server's own exit, and the
+/// bound is a backstop for a server that closed stdout while keeping stderr
+/// open (or left it open behind a child of its own).
+const STDERR_DRAIN_GRACE: Duration = Duration::from_secs(2);
+
 /// One configured MCP server, as `~/.future/mcp.json` describes it.
 #[derive(Debug, Clone, Serialize, Deserialize)]
 #[serde(deny_unknown_fields)]
@@ -379,14 +385,25 @@
             .ok_or_else(|| format!("server '{name}': stdout was not piped"))?;
 
         let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
-        if let Some(stderr) = child.stderr.take() {
+        let stderr_drain = child.stderr.take().map(|stderr| {
             let tail = Arc::clone(&stderr_tail);
-            tokio::spawn(async move { drain_stderr(stderr, tail).await });
-        }
+            tokio::spawn(async move { drain_stderr(stderr, tail).await })
+        });
 
         let mut session =
             Session::new(stdout, stdin, timeout).with_max_frame_bytes(max_frame_bytes);
         if let Err(error) = session.initialize().await {
+            // The tail is written by a task of its own, so nothing orders it
+            // before this read: a server that dies during its handshake is
+            // exactly the case where its own stderr is the explanation, and on a
+            // busy host this error path can run before the drain task has ever
+            // been polled. Join it first: it ends when the server's stderr
+            // reaches EOF, which the server's exit produces (the bound is a
+            // backstop for a server that closed stdout but kept stderr open, or
+            // left it open behind a child of its own).
+            if let Some(drain) = stderr_drain {
+                let _ = tokio::time::timeout(STDERR_DRAIN_GRACE, drain).await;
+            }
             let mut message = format!("the MCP server '{name}' failed its handshake: {error}");
             let tail = tail_of(&stderr_tail);
             if !tail.trim().is_empty() {
```

## 8.6 Gates (the supervisor's acceptance requirement)

| command | result |
|---|---|
| `cargo test -p future-cli -j 4`, 5 runs, worktree @ `de297bd9` | **4 pass, 1 FAIL(rc=101), 0 timeouts** — the red run's 2 failures are §8.7's two new items; **no mcp test and no readiness test failed in any of the 5 runs** |
| `cargo clippy -p future-cli --all-targets -- -D warnings` (worktree @ `de297bd9`) | **rc=0** |
| `cargo fmt -p future-cli -- --check` (worktree @ `de297bd9`) | **rc=0** |
| scratch tree **with §8.5 applied**: `cargo clippy -p future-cli --all-targets --no-deps -- -D warnings` + `cargo fmt -p future-cli -- --check` | clippy **finished clean**; fmt **rc=0**. (Without `--no-deps` the run stops on an unrelated `unused_imports` error in the *base revision's* `future-tui`, later fixed on the branch — not the patched crate.) |
| `bin-mcp-fixed`: `commands::mcp_servers commands::mcp_client` | 81 passed, 0 failed |

No `#[ignore]`, no `--skip`, no weakened assertion, no suite serialisation, no sleep used
as a fix, no test deleted, no state-changing git command.

## 8.7 Two reds the gate turned up (new items — handed to the supervisor for FLAKES.md)

Both failed in gate run 1 of 5 (the run that started at load 39; `1160 passed; 2 failed`).
Neither is in this unit's item list; both files are in its write set.

### 8.7.1 `doctor_captures_partial_version_from_hanging_binary` — 8/80 loaded (F-CLI-2 item 1's residual)

`cli/src/commands/doctor.rs:1076`, assertion `expected partial --version output to survive
the timeout`. This is **F-CLI-2's own new test** from §7.1.3, and it appeared 1/5 in this
unit's gate window and **8/80 (10 %)** in a 16-stream × 5-round rig at 32 burners
(load 8.5 → 49). The failing report shows the `Desktop` line as a **bare path** — the
`VersionProbe::NoVersion` rendering — while the fixture is a script that `echo`es its line
and only *then* touches the marker the test waits for, so the write is provably before the
kill the test arms. The kill is ordered; **the capture is not**: `get_binary_version`
(`doctor.rs:307-376`) spawns the two pipe reads on tasks of their own and classifies
whatever they have accumulated — the same "read a buffer a task of its own is filling"
family as §8.1.1. Two candidate paths, not yet split: (a) the captured buffer at the moment
of classification, (b) `is_version_candidate` rejecting the fixture's line. The check that
splits them is written and named: a `PROBE diag bin=… out_len=… out_err=… out=…` line at
the classification point (installed in the scratch copy of `doctor.rs`, then reverted),
which prints the raw bytes and any read error. **Two instrumented windows caught no
failure** (64 runs at 32 burners, 16 runs at 56 burners — the latter cut short by the
harness's 600 s bound), so the split is not in hand; the rate is.

FLAKES line (handing over, not editing): `doctor_captures_partial_version_from_hanging_binary`
— 8/80 under 32 burners + 1/5 gate — the marker orders the *child's write*, not the
*probe's capture*; same missing happens-before family as item 23, one layer out.

### 8.7.2 `create_page_times_out_when_target_never_discovered` — 1/5 gate, 0/80 loaded (candidate)

`cli/src/browser/chromium/chromium_page.rs:780`, `assertion failed: started.elapsed().as_secs() >= 5`.
The message assertion above it passed (`Target … not discovered within timeout`,
`chromium_page.rs:340`), so the production timeout fired but **returned in under 5 s of
monotonic elapsed time**. The budget at `:328` is `crate::utils::time::now_millis() + 5000`
— a **wall clock** (`SystemTime`, `cli/src/utils/time.rs:9`), while the assertion measures
`Instant`. A backwards wall-clock step (NTP) inside the loop is the only path I can see
that produces exactly this failure, and the assertion has no slack (a `4.999…s` elapsed
renders as `as_secs() == 4`). **Not reproduced**: 0/80 in a 16-stream × 5-round rig at 32
burners (5.02 s per run when it passes), 1/5 in the gate. Named check: print the loop's
deadline origin alongside the loop's exit time, or measure the loop with an `Instant` and
compare the two clocks over a run.

## 8.8 Commands, logs and results

Environment (every shell): `ulimit -n 10240`, `PATH=<pinned 1.97.0 toolchain>/bin:$PATH`,
`CARGO_HOME=~/.cargo`, `RUSTUP_HOME=~/.rustup`, `HOME` = the scratch/test home.

```bash
# scratch checkout (not a worktree add) and the two arms
git -C <worktree> archive HEAD | tar -x -C /tmp/fcli3/src
git -C <worktree> diff 76982a94..HEAD -- cli/src/commands/browser_tools.rs > revert-p2.patch
git -C <worktree> show 76982a94 -- cli/src/commands/browser_tools.rs > p1.diff   # then hunkfilter.py
cd /tmp/fcli3/src && git apply -R revert-p2.patch && git apply -R revert-p1.patch
CARGO_TARGET_DIR=/tmp/fcli3/target cargo test -p future-cli --lib --no-run -j 8

# item 1 rates
mcp-rig.sh <label> <runs> <binary>              # sequential single test
mcp-rig-par.sh <label> <rounds> <streams> <bin> # 16 concurrent single-test processes
mcp-loaded-ab.sh 48 20 16 bin-head bin-mcp-fixed

# item 2
target-rig.sh head   3 3 <bin-head>          # 3 streams x 3 rounds, 93 tests each
target-rig.sh before 3 3 <bin-before-fix>
loaded-ab.sh 48 readiness-rig.sh 8 8 bin-before-fix bin-head
control-rig.sh prefix-12s-launcher 5 bin-control-before
control-rig.sh fixed-12s-launcher  5 bin-control-after

# the supervisor's gates, from the worktree
bash .future/cov100/flake-rate.sh --label cli-fcli3-gate --runs 5 --timeout 420 \
    -- cargo test -p future-cli -j 4
cargo clippy -p future-cli --all-targets -- -D warnings
cargo fmt -p future-cli -- --check
```

Logs (outside the repo; nothing is committed from here): the gate's
`$TMPDIR/flake-cli-fcli3-gate/` (`cli-fcli3-gate-1.log` is the red run, with both §8.7
panics); the scratch tree `/tmp/fcli3/src`, its target `/tmp/fcli3/target`, the arm
binaries `/tmp/fcli3/bin-*`, and the rig dirs `/tmp/fcli3/rig-{mcp,target,ready,control,one}-*`
plus `/tmp/fcli3/{gate.log,clippy-scratch.log}`.

**Load while measuring** (so the numbers can be read in context): item 1's sequential 100
at 13.4; the 16-stream ambient window at 13.4 → 9.7; `bin-head` ambient at 6.9; the burner
windows at 12–68 (`mcp` 3/160), 12–53 (`mcp` 58/320), 9.7–69 (readiness 0/64 both arms);
the targeted rigs at 19.0 → 3.5 and 3.4 → 4.9; §8.7.1's rate at 8.5 → 49; the gate window
at 39 → 4.5.

# F-CLI-4 — the §8.5 patch landed and measured, the doctor split, the clock check, the two readiness siblings

Successor of F-CLI-3. Write set: `cli/src/commands/mcp_client.rs`,
`cli/src/commands/doctor.rs`, `cli/src/browser` (incl. `chromium`, `safari`),
`cli/src/commands/browser_tools.rs`, this file.

| item | where | verdict |
|---|---|---|
| (a) MCP handshake stderr (FLAKES 23) | `cli/src/commands/mcp_client.rs:404` (assert `mcp_servers.rs:1982`) | **§8.5 landed and re-measured in this run: 37/320 before → 0/320 after**, both arms inside one 48-burner window, back to back. All 37 before-failures carry the one recorded signature |
| (b) `doctor_captures_partial_version_from_hanging_binary` (FLAKES 25) | `cli/src/commands/doctor.rs:1013` (assertion `:1091`) | **split, mechanism named, fixed.** Deterministic control: fixture that writes after 25 s → pre-fix arming **4/4 red** (bare path, 25.4–26.4 s), fixed arming **4/4 green** (55.4–55.8 s). Landed: **0/48** loaded at 48 burners |
| (c) `create_page_times_out_when_target_never_discovered` (FLAKES 26) | `cli/src/browser/chromium/chromium_page.rs` (budget `:333`, assertion `:786`) | **named clock-origin check run, both arms: 0/48 + 0/48, the two clocks within 1 ms in every run — the candidate did not reproduce. Production read fixed to `Instant` anyway**, so the budget's clock and the assertion's clock are now the same clock |
| (d) the two readiness siblings | `browser_tools.rs:3685`, `safari/safari_manager.rs:479` | **both reproduced with the §8.3.3 forcing device and fixed: 5/5 red → 5/5 green each, same force.** Landed: **0/24 each** loaded at 48 burners. One measurement in this item also caught a flake I had just introduced (§9.4.2) |

## 9.0 Run identity and environment

| field | value |
|---|---|
| revision at the start | `9f445f3e` (branch `test/cov100-r2`); the branch moved under me while measuring (peer units' commits, incl. one `git add -A`-style snapshot that swept my in-flight `cli/` edits into a commit — verified by `git diff 9f445f3e -- cli/…`, which shows only my hunks) |
| host | macOS (darwin, arm64), 32 CPUs; ambient load 4–30, raised deliberately to 48–77 with 48–56 synthetic CPU burners where stated |
| toolchain / env | pinned `1.97.0-aarch64-apple-darwin`; `ulimit -n 10240`; `HOME=/tmp/fcli4/test-home` |
| binaries (this run) | `/tmp/fcli4/bin-before` (unpatched), `bin-after` (a), `bin-diag` (b, instrumented), `bin-ctrl-before` / `bin-ctrl-after` (b control), `bin-clock-before` / `bin-clock-after` (c), `bin-sib-before` / `bin-sib-after` (d), `bin-final` (landed) |
| rigs / logs | `/tmp/fcli4/*.sh`, `/tmp/fcli4/rig-*` (outside the repo; nothing committed from there) |

Rates are counts of whole single-test processes, 16 (or 8) concurrent per round; the
burners are held identical across the two arms of each comparison.

## 9.1 Item (a) — the §8.5 patch, landed and measured

The patch is now in the tree (`STDERR_DRAIN_GRACE` at `mcp_client.rs:67`, the join at
`:404`). Both arms were built from *this* worktree at this revision, in this run:

```bash
cargo test -p future-cli --lib --no-run -j 8        # bin-before: unpatched tree
# edit cli/src/commands/mcp_client.rs (§8.5)
cargo test -p future-cli --lib --no-run -j 8        # bin-after
bash /tmp/fcli4/mcp-ab.sh 20 16 /tmp/fcli4/bin-before /tmp/fcli4/bin-after   # 48 burners, held for both arms
```

| arm | runs | failures | load during the arm | single test process |
|---|---|---|---|---|
| A `bin-before` (unpatched) | 320 | **37 (11.6 %)** | 16.44 → 43.96 | ≈1 s |
| B `bin-after` (patched) | 320 | **0** | 43.96 → 53.89 (same burners still up) | ≈1 s |

The after arm ran at the higher load of the two. Every one of the 37 failing logs is the
same datum:

```
panicked at cli/src/commands/mcp_servers.rs:1982:13:
the MCP server 'broken' failed its handshake: the MCP server closed its output before answering 'initialize'
```

which is §8.1.1's signature (no `--- server stderr` section). My before-rate is 37/320
where F-CLI-3's was 58/320 on another binary and another load window — same shape, same
command, same `-j`-free single-test rig, so the *range* is the honest summary: 11.6 % vs
18 %, and 0/320 in both after-columns. `commands::mcp_*` (81 tests) passes on the patched
binary; clippy/fmt clean (§9.5).

## 9.2 Item (b) — the split, the mechanism, the deterministic control

### 9.2.1 The split check (written, used, then removed)

Two prints, both `FUTURE_PROBE_DIAG`-gated and both removed before landing — the named
check the todo asked for, one layer each:

* at the classification point of `get_binary_version`:
  `PROBE diag bin=… spawn_ms=… budget_ms=… kill_ms=… trywait_exit_ms=… budget=… out_len=… err_len=… out=… err=…`
  (the raw bytes the classification is actually fed, plus both read tasks' outcomes);
* in the armer: `ARM diag marker_seen=… waited_ms=… wall_ms=… arming_zero=…`.

What they show on healthy runs (6 logs, one per build stage):

```
ARM diag marker_seen=true waited_ms=384 wall_ms=1792167111669 arming_zero=true
PROBE diag bin=…/fake-bin/future-desktop spawn_ms=1792167111298 budget_ms=Some(1792167111716) kill_ms=Some(1792167111717) trywait_exit_ms=None budget=Some(0ns) out_len=53 out="FutureOS: agent already reachable at 127.0.0.1:50051\n"
```

So on a healthy run the order is spawn → (fixture writes, touches marker) → arm →
kill, and the line is captured: **the classification path (candidate b in FLAKES 25) is
not the failure path** — the fixture's line is `Captured` whenever it is in the buffer,
and two instrumented windows (mine: 80 runs; F-CLI-3's: 64 + 16) caught no natural failure.

One asymmetry worth recording, because it is what makes "empty buffer" a signal at all:
the reader tasks cannot fail — each returns its buffer unconditionally and `copy`'s error
is swallowed (`let _ = tokio::io::copy(…)`) — so an empty classification input means
*nothing was ever readable from the pipe*, not "the read went wrong". That is exactly the
kill-before-write state, and it is why the empty-buffer reds and the fallback arming are
one mechanism rather than two candidates.

### 9.2.2 The mechanism

The test's deadline seam is a **process-global** `PROBE_BUDGET_OVERRIDE` read per loop
iteration, and it is armed by a *wall-clock* fallback whose expiry is unconditional:

```rust
while !written.exists() && now < deadline_20s { sleep(20ms) }   // the safety valve
*PROBE_BUDGET_OVERRIDE = Some(Some(ZERO));                       // "the deadline has passed"
```

The marker is written by the fixture *after* its line is in the pipe, so arming off the
marker is ordered with the write — but the **fallback arming is not ordered with the probe
it means to interrupt at all**. If it fires before the desktop probe's child has written
(the host took >20 s to reach the desktop check, or to run the fixture's script), the
`ZERO` budget is picked up by an in-flight probe whose child has produced nothing, the
product kills it before the write, the classification sees an empty buffer, the Desktop
line renders as a bare path, and the test fails while the product behaved correctly.

Two independent pieces of evidence pin the fallback as the path (I could not catch it
naturally — see §9.2.4 — so this is inference plus a control, not a rate):

* the recorded failing log's run takes **20.07 s** (`finished in 20.07s`) and the armer's
  20 s bound is the only 20 s clock anywhere in this flow; a *passing* run takes **30.4 s**
  (measured 3× here), because the fixture's `sleep 30` keeps the pipe open after the kill —
  i.e. a red run ends almost immediately after the arming, which is only possible if the
  child had not started `sleep 30`, i.e. was killed before/at the start of the script;
* a killed-before-write child produces exactly the recorded rendering, and the fixture's
  line is otherwise always classified `Captured` (§9.2.1).

### 9.2.3 The fix

Arm only when the marker has been seen (proof that the desktop probe's child wrote), and
when the bound expires leave the probe on its own budget instead of arming:

```rust
let marker_seen = written.exists();
if marker_seen { *PROBE_BUDGET_OVERRIDE.lock().unwrap() = Some(Some(Duration::ZERO)); }
marker_seen
```

The degradation is deliberate and bounded: if the marker never appears, no kill is armed,
the probe waits for the child to end (the fixture's own `sleep 30` bounds it), and the
assertion reports what the probe actually captured — a real red with real evidence instead
of a false one. The normal path is unchanged and still exercises the kill: the diag shows
`arming_zero=true`, `budget=Some(0ns)`, `kill_ms` present and `out_len=53` on every
healthy run, and the assertion message now names whether the fixture wrote.

### 9.2.4 The deterministic control (the forcing device) and the rate window that did not fire

The same test, with a fixture that does not write until 25 s (> the armer's bound), on both
constructions — one line in the fixture, everything else untouched:

| construction | runs | failures | per run |
|---|---|---|---|
| pre-fix arming (`ZERO` unconditional on expiry) + 25 s fixture | 4 (sequential) | **4** | 25.4–26.4 s; `Desktop` line = bare path; the exact assertion message |
| fixed arming (marker-conditional) + the same 25 s fixture | 4 (sequential) | **0** | 55.4–55.8 s (probe waited for the child's exit and captured the line) |

The rate window that did **not** reproduce it, stated plainly: **0/80**, 16 streams × 5
rounds, 48→56 burners, load to 77.0, with the instrumentation installed. What that window
did produce is the margin, which is the useful number: the marker was seen **435 … 6334 ms**
after each test started (64 values; median ≈ 3.2 s) against a 20 s fallback — i.e. the
fallback needs the host to be ~3× more stalled than this window could make it. (F-CLI-3's
two instrumented windows were 0/64 and 0/16; the recorded 8/80 came from the campaign's
13-worker mixed load, which I could not recreate.)

Landed artifact: **0/48** (16 streams × 3 rounds, 48 burners, load → 67.9) plus the full
lib suite green.

## 9.3 Item (c) — the named clock-origin check

The check prints the deadline origin beside the loop exit **in both clocks**, on both
constructions (pre-fix wall-clock loop, post-fix `Instant` loop), 48 runs each at 32
burners:

```
CLOCK diag origin wall_ms=1792167234463 origin_instant_elapsed_ms=0 exit wall_ms=1792167239491 exit_wall_delta_ms=5028 exit_instant_delta_ms=5027
```

| arm | runs | failures | exit Δ wall (ms) | exit Δ `Instant` (ms) |
|---|---|---|---|---|
| before (wall-clock loop, pre-fix) | 48 | **0** | 5002 … 5051 | 5001 … 5051 |
| after (`Instant` loop, post-fix) | 48 | **0** | 5009 … 5041 | 5009 … 5041 |

**The two clocks agreed within 1 ms in every one of the 96 runs**, so the clock step the
candidate needs (a ≥5 s *forward* wall step inside a 5 s window: NTP step, or a host
suspend/resume, since macOS `Instant` does not count suspend) did not happen here — the
candidate did not reproduce, on top of F-CLI-3's 0/80 on the un-modified construction.

The failure signature does require it: the loop's exit was `now_millis() < deadline`, and
a forward step makes that false while `started.elapsed()` (the assertion's clock) still
reads < 5 s, which is exactly "message assertion passed, `elapsed().as_secs() >= 5` failed".
So the production read is now the same clock the assertion measures:

```rust
let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
while std::time::Instant::now() < deadline { … }
```

Rejected alternative: re-registering the row with the reason and leaving the wall clock —
two clocks that can disagree is a removable production defect with a two-line change and
no behaviour difference on a monotonic host, so the fix is the cheap half.

**Residual, not touched (flagged, outside this item):** the same wall-clock-budget class
exists at `cli/src/browser/backend.rs:16,24,28` and
`cli/src/browser/chromium/chromium_navigation.rs:167,214` (`now_millis()` budgets). Each
has its own test that would need its own check; F-CLI-4's item names the `create_page`
path only.

## 9.4 Item (d) — the two readiness siblings, with real before/after

### 9.4.1 The forcing device on both (the §8.3.3 method)

One line in each arm's launcher/driver script — `sleep 12`, longer than the product's 10 s
readiness window — everything else untouched. Same binary pair, same command, sequential:

| test | before (current construction) | after (fixed construction) |
|---|---|---|
| `commands::browser_tools::tests::start_launch_becomes_reachable_reports_started` | **5/5 red** (10–12 s): `assertion left: "starting", right: "started"` | **5/5 green** (0–3 s — the launcher is off the critical path) |
| `browser::safari::safari_manager::tests::launch_then_become_reachable_reports_started` | **5/5 red** (12–18 s): `a driver that answers in the window is a success: "safaridriver did not respond at http://127.0.0.1:56867 within 10s."` | **5/5 green** (12–16 s: the poll keeps asking until the mock appears) |

### 9.4.2 The fixes

* **chromium sibling** — it keeps the `executablePath` route (which the override-based test
  above does not cover) but the launcher's own start is no longer on the path: the script's
  only job is to be spawned, and the endpoint is **the test's own listener**, bound once the
  launch has committed (the profile directory — the CLI's own pre-spawn step), with the poll
  under `ReadinessBudgetGuard::set(40)`. The landed test **keeps the 12 s launcher** as a
  standing guard: it costs nothing (the test does not wait for the launcher) and it is the
  same force that used to make it red.
* **safari sibling** — a probe-count stopping rule for `safari_start`'s poll
  (`readiness_budget()`, mirroring `browser_tools`), so what ends the wait is "the poll
  noticed the answer" rather than the 10 s window; the window keeps its own test
  (`start_launch_never_ready_times_out`, which asserts the full 10 s wait and the
  `did not respond … within 10s` error). The landed driver script stays a plain `touch`.
* **one thing I got wrong, and the measurement that caught it** — my first attempt kept the
  12 s driver script in the *landed* safari test as a permanent guard. At 8 concurrent
  streams under 48 burners that is **8/24 red**, and every one is
  `the driver was never spawned, so the poll was never reached`: the 12 s script sits on
  the test's *own* 20 s marker bound, i.e. I had converted one host dependence into another.
  The force belongs in the control arm; the landed test carries the fast script (FLAKES
  rule 4b, again).
* `spawn`-ordering subtlety both fixes respect: an endpoint that answers *before* the
  spawn takes the `already_running` branch, so the mock cannot be up front — the anchor is
  the product's own commit step (profile directory / the driver's spawn), not a sleep.

Landed artifact at 48 burners: chromium sibling **0/24**, safari sibling **0/24**.

## 9.5 Gates

| command | result |
|---|---|
| `cargo clippy -p future-cli --all-targets -- -D warnings` | **rc=0** |
| `cargo fmt -p future-cli -- --check` | **rc=0** |
| `cargo test -p future-cli --lib -j 4` | **1162 passed; 0 failed; 0 ignored** |
| `.future/cov100/flake-rate.sh --label fcli4-gate --runs 1 --timeout 500 -- cargo test -p future-cli -j 4` | **rc=0, pass**: lib 1162 passed / 0 failed + the integration targets (16, 2, 2, 1, 0 passed; 0 failed), run under the peers' own load |
| loaded windows on `bin-final` (48 burners, identical burners across all four) | doctor **0/48**, mcp **0/32**, chromium sibling **0/24**, safari sibling **0/24** |
| `commands::mcp_*` on the patched binary | 81 passed, 0 failed |

No `#[ignore]`, no `--skip`, no weakened or deleted assertion, no suite serialisation, no
widened production timeout, no sleep used to paper over a window (the 12 s lines are
fixture/forcing devices that make a dependence *visible*). The only production behaviour
changes are (a) joining the stderr drain on a failed handshake — bounded by the server's
own exit — and (c) a monotonic budget; plus the two test-only seams.

## 9.6 What I could not do

* **(b) no natural reproduction**: 0/80 in a 48→56-burner window with instrumentation
  (F-CLI-3: 0/64 and 0/16). The mechanism is therefore proved by the deterministic control
  (§9.2.4) plus the recorded log's 20.07 s duration, not by a rate.
* **(c) never reproduced** (0/96 in this unit, 0/80 in F-CLI-3). The check ran and found
  no divergence; the fix removes the divergence by construction.
* **(a)'s after-rate** is from a binary built in this run at 48 CPU burners — CPU
  oversubscription, not the campaign's mixed load (13 workers, disk pressure, the volume at
  95–98 %), which is the load shape that produced the recorded 58/320.
* The `cli/` tree moved under me during the run (a peer's snapshot commit swept my in-flight
  edits into a commit); every number above is from binaries I built from this worktree, and
  the landed hunks were re-verified with `git diff 9f445f3e -- cli/…`.

## 10. F-CLI-5 — the remaining wall-clock budgets in `cli/src/browser` (supervisor-verified record; the unit's run ended before it wrote this)

**Provenance, stated plainly because I did not run this measurement.** Unit `worker-flake-cli`
(todo `todo_84fc86918492`, run `run_08acf2f9decc47f2855ce6f074d580bf`) instrumented the sites,
built its own test binaries and ran the check; its run then ended and it recorded nothing —
neither this record nor its `todo_completed` evidence (which is a four-sentence plan fragment).
I reconstructed the measurement from its own artifacts and verified every number below myself:

* `/tmp/fcli5/clock-check.sh` — the driver (streams × rounds, `FUTURE_CLOCK_DIAG=1`, appends
  every `CLOCK` line, prints each round's red count **and the load it ran at**).
* `/tmp/fcli5/clock-backend-diag/{summary.txt,all.log,r*/*.log,*.rc}` — the results.
* `/tmp/fcli5/pristine/{backend.rs,chromium_navigation.rs}` — pre-instrumentation backups,
  byte-identical to the revision before the snapshot that committed the experiment (checked).

### 10.1 What was measured

Command shape: `clock-check.sh backend-diag <streams> 20 <test-threads> <bin> browser::backend::tests::deadline_semantics`.

| quantity | value |
|---|---|
| rounds | **20**, `reds=0` in every round (`*.rc` all zero — I re-summed them) |
| load during the window | 1-min **50.87 in round 1 → 62.91 in round 20** (the unit's own 48 burners, see 10.3) |
| `CLOCK` samples captured | **5102** lines |
| sites exercised | `deadline:new` 1277, `deadline:expired` 1274, `deadline:exit` 2551 — i.e. `cli/src/browser/backend.rs` 16/24/28 only |
| wall-vs-Instant divergence | **0 ms** in 3205 comparable samples, **1 ms** in 370 (the instrumentation's own ms granularity), never more |

### 10.2 What this settles, and what it does not

* **Settled: `backend.rs:16/24/28` carry no clock-origin fault.** A wall-clock/`Instant`
  mismatch is the only mechanism that would make those budgets drift, and at worse-than-CI
  load (1-min 50–63 on this host) it never happened beyond the 1 ms that the two clocks
  cannot resolve. Combined with `reds=0` over 20 rounds, the honest classification for these
  three lines is **waived by measurement** — the same conclusion §9.3 reached for
  `chromium_page.rs:333`, now with a load-stamped rate behind it rather than an argument.
* **NOT settled: `chromium_navigation.rs:167,214`.** Those sites were instrumented but the
  driver's filter (`browser::backend::tests::deadline_semantics`) never entered them — the
  `nav-phase1:*` and `wait-until:*` sites appear **zero** times in `all.log`. So two of the
  five lines this ticket named remain **unmeasured**; they are not waived, they are open.
* The sweep's second half ("re-run the cli repeat-run sweep on the frozen revision") was not
  done at all.

### 10.3 Two things this run left behind, and what the supervisor did about them

1. **The load outlived the run (F.65).** `/tmp/fcli5/burn.sh` starts `n` `timeout <secs> sh -c
   'while :; do :; done'` burners (default 48/600 s; this batch ran **48 × 2400 s**, pid file
   `/tmp/fcli5/burn/pids`). Its run ended at ~02:15 and the burners kept running, holding 1-min
   load at **71–82 on 32 CPUs** while three peer units (app-data segment 8/9, headless TAU-D3,
   shell TAU-C7) were mid-measurement. I verified all 48 pids in that file were burners
   (48/48 matched, 0 mismatches) and stopped them; load fell 68 → 53 within 20 s. Their
   bounded `timeout` would have cleared them ~19 minutes later, but not before those peers
   finished measuring. **This is the F.65 incident's actual source** — it was the campaign's
   own unit, not an outside process.
2. **The instrumentation was committed (F.67).** `cli/src/browser/backend.rs` gained a
   `start_instant` field and three `eprintln!` sites, and `chromium_navigation.rs` gained two
   more, all marked `TEMP DIAG (F-CLI-5)`. The run ended before reverting them, and a
   supervisor snapshot committed them into HEAD — after which `git status` read **clean** and
   only history knew. I reverted both files to the pristine bytes (verified: `git diff` against
   the pre-snapshot revision is empty), `cargo check -p future-cli` passes, and the marker list
   in `endgame.sh` step 0b and the new `snapshot.sh` guard now include this marker, with a
   control showing the old list missed the real commit (0 hits) and the new one catches it
   (17 hits, excluding `.future/` so the checker does not match its own prose).

### 10.4 Verdict

Recorded as **rework**: the measurement it obtained is real and is now written down here, but
the ticket's deliverable (sweep *all* the named budgets, fix or waive each with its consumer,
then re-run the repeat sweep) is **partially** done — two of five lines unmeasured, no
repeat-run sweep, no record from the unit, and an experiment left in the tree. The successor
ticket carries exactly those three items; it must not re-do 10.1, which is settled.
