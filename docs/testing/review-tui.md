# REV-TUI — adversarial review of `docs/testing/module-tui.md` §9.4 (24 residual lines)

Review unit **REV-TUI** (`todo_3d0a510da952`) · module **future-tui** (`tui/`) · the
unit under review is frozen (`todo_ee1f7a6715a0`, supervisor-verified: 270 → 24,
99.9601 %, tripwire 24/24) · reviewer session `20261016-190808-dc590d77cf6846e385b6caff60fe5637`.

Host: macOS arm64, rustup `rustc 1.97.0` (pinned toolchain), **load average 19–40**
during the review (eight campaign workers plus mine). No write inside the campaign
worktree except this file; every probe ran in a scratch checkout (§6).

| | |
|---|---|
| reviewed record | `docs/testing/module-tui.md` §9.4 — sha256 `8ef58182842b612e3d78ae81023b4179ad5f46748f9b5e7d7cad09285af0bf0f` |
| frozen sources reviewed (worktree) | `agent_supervisor.rs` `f2bc65c2…`, `app.rs` `04350328…`, `terminal.rs` `d54e4679…`, `rpc/grpc_client.rs` `3b1d7499…`, `components/chat_area.rs` `33e853fb…`, `update.rs` `d73a9d65…`, `index.rs` `4f0f5b48…` (full hashes in §6.1) |
| record revision | base `8449acf6608849c792547b83e66cdb8d9d7a8b25` + this unit's `tui/` sources (frozen) |

## 0. Verdict roll-up

**11 lines FELL, 13 SURVIVED, 0 UNVERIFIABLE.** Six FELLs carry a covered-line
counter as evidence (the line is no longer `DA:0` in the probe report), four more
carry a deterministic probe that executes the line, and one (`agent_supervisor.rs:121`)
carries a controlled with/without experiment that reproduces both of the campaign's
measured forms bit for bit.

| verdict | lines |
|---|---|
| **FELL** (11) | `agent_supervisor.rs:121`; `app.rs:11995`, `25296`, `7220`, `7221`, `7222`; `grpc_client.rs:1239`, `1344`, `3483`, `3828`; `terminal.rs:1283` |
| **SURVIVED** (13) | `terminal.rs:1206`, `1216`, `1217`, `1276`, `1277`, `1280`; `app.rs:3610`; `agent_supervisor.rs:141`, `142`, `143`, `267`; `chat_area.rs:2651`; `update.rs:104` |

Two things the record gets right and this review confirms independently:

* the headline figure is reproducible from the artefact the record cites:
  `python3 .future/cov100/covlines.py total /tmp/tui-lcov-final6.info --under .worktrees/cov100-r2/tui/src`
  → `covered=60133 measured=60157 uncovered=24 lines%=99.9601`, and `detail` prints
  exactly the 24 lines of §9.4, file for file — the ledger is not stale against
  `tui-lcov-final6.info`;
* every row's line number matches the frozen source, and the quoted code is verbatim.
  The *numbers* are right; two of the four **categories** leak
  (`unreachable-in-this-environment` and `unreachable-by-construction`).

The 244-line harness story, §9.1–§9.3, and the `tmux-diff.sh` rendering gate were
outside this review's scope (the todo names §9.4's 24 lines); §7 says what I did not
independently re-run.

## 1. Method (REVIEW.md, followed as written)

Scratch checkout, never `git worktree add`, never a write in the campaign worktree:

```bash
mkdir -p /tmp/rev-tui && cd /Users/geilige/future-os/.worktrees/cov100-r2
git archive 8449acf6 | tar -x -C /tmp/rev-tui
rsync -a --delete --exclude target tui/ /tmp/rev-tui/tui/     # the frozen sources
```

Per row: read the code, enumerate the real call sites myself, then **construct an
input** that reaches the line. Counterexamples that need a test were written as probe
tests inside the scratch tree's own `mod tests` blocks (the helpers are private to
those modules) and run with `cargo test` / `cargo llvm-cov`; the `DA` counter of the
reviewed line in the probe report is the covered-line evidence. Probe sources, hashes
and exact commands are in §6 so this review can be verified rather than trusted.

**Criterion for FELL vs SURVIVED** (stated because it decides several rows): a row
**falls** if a test can be written that executes the line *deterministically, green,
with an assertion on the behaviour that line implements* (PLAN §4). A "test helper's
failure arm" therefore survives only when **no** input can make it fire while the
suite stays green — "a test cannot assert its own failure" is not sufficient, because
a *caller* can assert the helper's contract. Rows where the covering test would be
low-value are still reported as FELL, with the value judgement separated out (§8).

## 2. Every row, verdict and evidence

| path:line | §9.4 category | verdict | what I built / why it holds |
|---|---|---|---|
| `agent_supervisor.rs:121` | unreachable-in-this-environment | **FELL** | controlled experiment: a same-tree sibling `future` candidate makes the launched agent answer on the 12th probe → `DA=1` (was `0`); the campaign's own workspace form shows the same (`ws-dirty` `DA=1`). §3.1 |
| `agent_supervisor.rs:141,142,143` | unreachable-in-this-environment | SURVIVED | `Child::try_wait()` → `Err` needs the OS to refuse waiting on a child this process spawned and still owns; no input constructs it (no `waitpid` failure path, no external reaper). Probe run (no counterexample attempted): `DA=0` in `lcov-lib-probes2.info`, i.e. my probes did not reach it by accident either |
| `agent_supervisor.rs:267` | unreachable-by-construction | SURVIVED | the test's `Ok(_) => panic!` fires only if `ensure_agent_running("127.0.0.1:1")` returns `Ok` — that needs an agent on port 1 answering a readiness handshake with `EXPECTED_AGENT_VERSION`; binding port 1 needs root. The category is right (premise arm), the mechanism sentence ("`cfg!(test)` blocks the launch path") is secondary — the first gate is `select_agent`'s "no local fallback" |
| `app.rs:3610` | attribution-artifact | SURVIVED | verified from `final6` itself: `3601–3609 DA=1`, **`3610 DA=0`**, `3611 DA=1`, `3613 DA=77` — a closing brace whose block demonstrably ran (`first_attempt = false` at 3609 has a live reader, and `wait_for_agent_retries_until_agent_appears` asserts both messages) |
| `app.rs:7220,7221,7222` | unreachable-in-this-environment | **FELL** | integration tests link the lib **without** `cfg(test)`, so the real-platform twin is compiled into `tui/tests/*`; `App::handle_input` + the `/sandbox` command are public API → `DA=1` for all three lines, no PTY and no agent needed. §3.3 |
| `app.rs:11995` | unreachable-in-this-environment | **FELL** | the 120 s bound is 4 800 *tokio* sleeps; `tokio`'s `test-util` (paused clock) is **already a dev-dependency of this crate** → `#[tokio::test(start_paused = true)]` walks the whole budget in milliseconds → `DA=1`, `11993 DA=4800`. §3.2 |
| `app.rs:25296` | unreachable-in-this-environment | **FELL** | the helper's doc comment *documents* `None` as its outcome; a 20 s test with nothing delivered reaches it and asserts it → `DA=1`, `25294 DA=886`. §3.2 |
| `terminal.rs:1206` | unreachable-by-construction | SURVIVED | the arm fires only if the seam does not fire, and the seam is set two instructions earlier under `terminal_test_lock()`; no input flips that. I checked the discipline: **every** test that constructs a `Terminal` takes the same lock, so the process-global one-shot is not exposed today (§5.3). Probe run: `DA=0` |
| `terminal.rs:1216,1217` | unreachable-in-this-environment | SURVIVED | POSIX `Backend::new()` is infallible **and** the one-shot seam was consumed by the first construction in the same test; Windows is the OS that can fail here. Probe run: `DA=0` |
| `terminal.rs:1276,1277,1280` | platform-unmeasured | SURVIVED | verified in the source, not assumed: `terminal_posix.rs:250–301` can only return `Timeout`, `Input` or `Signal(_)` — `ReadWait::Resize` is the Windows console event. POSIX asserts the same edge via `Signal(SIGWINCH)`; `TestPty::resize` covers the reader path |
| `terminal.rs:1283` | unreachable-by-construction | **FELL** | the invariant is a *timing* argument: the polling test itself can supply the signal (`raise(SIGWINCH)` lands a byte in the self-pipe, the next `wait(0)` returns `Signal(28)`, and the helper panics) → `DA=1`, panic text `unexpected wait outcome: Signal(28)`; and an **unlocked** sibling test mutates the same global signal pipe, so the arm is race-reachable too (§3.6, §5.5) |
| `grpc_client.rs:1239` | unreachable-in-this-environment | **FELL** | four addresses the waiver did not consider; three take this arm → `DA=3`. §3.4 |
| `grpc_client.rs:1344` | unreachable-in-this-environment | **FELL** | "a stop that lands without an outstanding notify is a race the suite cannot produce" — the suite already produces it: `subscribe_loop_top_stop_check` stores the flag with no notify, and the reconnect-poll twin is 12 lines away → `DA=1`. §3.5 |
| `grpc_client.rs:3483` | unreachable-by-construction | **FELL** | `attach`'s panic with nothing serving the address: bounded (20 × 25 ms), green under `#[should_panic]` → `DA=1` |
| `grpc_client.rs:3828` | unreachable-by-construction | **FELL** | "a test cannot assert its own failure" — a caller instead: `wait_for_backoff_above(&client, u64::MAX)` returns `None` in 4 s and the test asserts it → `DA=1` |
| `chat_area.rs:2651` | unreachable-by-construction | SURVIVED | the line is the *format argument of an `assert!` message*: it is evaluated only when the assertion is about to fail, i.e. only by a test that must fail. Correct category, correct mechanism. Probe run: `DA=0` |
| `update.rs:104` | unreachable-in-this-environment | SURVIVED | `check()` reaches it only for a *published-channel* version (`manifest_url` returns `Some`), and the URL is the production release endpoint. Covering it needs a numbered/release build **plus** a fetch to `dl.future-os.cn` (or a root-level hosts override); no assertion on the line's behaviour is possible without the network. The waiver's own pre-emption is sound; one extra reason to record: with `FUTURE_VERSION` set to a published label, the sibling assertion in `index.rs` (`this build has no published channel`) would fail, so the build configuration is itself load-bearing. Probe run: `DA=0` |

## 3. The falling rows in detail

### 3.1 `agent_supervisor.rs:121` — the highest-value verdict (FELL)

`Ok(_) => return Ok(Some(OwnedAgent { child }))` is registered
`unreachable-in-this-environment` with the mechanism *"no such binary exists in the
test environment (the real `future agent` is not built or installed by the test run)"*.
Both halves of that sentence are false in this environment, and the line is reached by
an in-tree test that predates the unit.

**The trigger.** The launch loop is in the *non-test* build of the library
(`cfg!(test)` returns early), so only a test target that links the lib without
`cfg(test)` can drive it. Two of this crate's integration targets do: the unit's new
`tui/tests/agent_sidecar_launch.rs`, and the pre-existing `tui/tests/cli_smoke.rs`
(`bughunt_regressions.rs` links the same way but spawns no process at all — checked:
no `Command`, no `current_exe`). Attribution probe (recording stub on the candidate
path, one test at a time):

```
--- cli_smoke::help_flag_prints_usage_and_exits_zero            spawns=0
--- cli_smoke::version_flag_prints_version_and_exits_zero       spawns=0
--- cli_smoke::unknown_option_exits_one                         spawns=0
--- cli_smoke::print_mode_without_message_exits_one             spawns=1
     argv: [agent] [--grpc-addr] [auto]
     ppid_cmdline=/tmp/rev-tui/target/debug/future-tui --print
--- cli_smoke::list_models_without_agent_exits_one              spawns=0
```

So the covering test is **`cli_smoke::print_mode_without_message_exits_one`**: it runs
the real `future-tui` binary, which reaches `index.rs:1099` → `ensure_agent_running`
before it ever looks at `--print`. Which candidate it spawns is decided by the
*measurement form* and by `PATH`.

**Controlled experiment** (`--test cli_smoke`, instrumented, same code, only the
sibling candidate differs — the sibling is what a `--workspace` run builds for free):

| counter (`tui/src/agent_supervisor.rs`) | with sibling `future` (≈workspace form) | without it (crate-scoped form) | campaign `ws-dirty-lcov.info` | campaign crate-scoped (`final6`) |
|---|---|---|---|---|
| `105` spawn match | 1 | 3 | 1 | 7 |
| `107–109` spawn failure | **0** | **2** | **0** | **5** |
| `112` inner loop entered | 1 | 1 | 1 | 2 |
| `113–119` probe | 12 | 2 | 2 | 5 |
| `121` **success** | **1** | **0** | **1** | **0** |
| `125` transport/unverified | 11 | 1 | 1 | 4 |
| `126–129` hard error | 0 | 1 | 0 | 1 |
| `140` backoff sleep | 11 | 1 | 1 | 3 |
| `index.rs:1130–1133` (`--print` usage) | **1** | **0** | 1 | 0 (unit tree: 5) |

Commands: §6.2. The without-sibling column reproduces the campaign's own
crate-scoped pattern exactly (spawn failures 2, hard error 1, `121`=0) — and its
hard error is the host's PATH binary: `/opt/homebrew/bin/future` is
**`future v0.0.2-8449acf6+local`** (mtime today 17:10), whose agent reports
`0.0.2-8449acf6+local` against this client's `0.0.0`, so `connect_verified` fails with
`Incompatible Future Agent: … local startup is forbidden` and the loop takes the
*hard-error* arm at 126–129. With a same-tree sibling the agent answers (12th probe;
the workspace run needed 2) and **line 121 executes**.

**What that means for the waiver.** The line is not unreachable here on any reading:
the campaign's own workspace-form baseline already counts it `DA=1`, and the covering
event is an in-tree test, not an artefact. The honest entry is one of:

1. **drop the row and record it as covered-in-the-workspace-form** (`ws-dirty-lcov.info`,
   `agent_supervisor.rs:121 DA=1`; the crate-scoped form is where it reads `0`);
2. or cover it in the unit's own form: the crate-scoped form fails only because the
   private target dir has no sibling `future`. A `--workspace` build puts one there;
   a hand-written fixture can too (a *sleeper* stub on the candidate path plus a
   readiness-serving mock on the local endpoint — `future_rpc::proto` + tonic are
   available to this crate, and the lib's own `AppMockAgent` is the shape to copy).
   There is no public mock-readiness server in `future_rpc` (checked: only
   `connection::validate_readiness`, a client-side function), so the fixture is work,
   not a one-liner — but it is not an environment impossibility.

Either way **the campaign total moves**: the workspace form counts this line as
covered, so "24 waived" is 23 in that form. See also §5.1 (why the crate-scoped
number is host-state dependent) and §5.2 (the exit-code coincidence that hides the
difference from `cli_smoke`).

### 3.2 `app.rs:11995` and `app.rs:25296` — the deadline rows (both FELL)

Both waivers rest on a *bound* the author believes a test cannot afford: "120 s at
25 ms" and "after a 20 s deadline". Neither is an environment limit.

```rust
    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "the history page never settled")]
    async fn probe_history_budget_is_reachable_under_a_paused_clock() {
        let (mut app, mut op_rx) = make_app(100, 30);
        app.history_paging.loading = true;          // the page never answers
        pump_until_history_settled(&mut app, &mut op_rx).await;
    }
```

`PUMP_BUDGET_ITERS = 4_800` is 4 800 **`tokio::time::sleep`** calls, so the paused
clock (tokio `test-util`, already a dev-dependency of this crate for the heartbeat
test) walks the entire budget in *milliseconds* of wall time, and the panic line is
executed. Measured: `11993 DA=4800` (the whole budget was walked), **`11995 DA=1`**.

For `wait_for_set_cwd`, its own doc comment states the contract ("`None` on timeout:
the caller's assertion carries the diagnostic"), so a test that delivers nothing and
asserts `sent.is_none()` is an *asserted* outcome — it costs 20 s of wall time, not
an impossibility. Measured: `25294 DA=886`, **`25296 DA=1`** (20.0 s).

*Mechanism correction:* the stated invariant ("the mock always answers" / "the live
fixture always delivers `/cwd`") describes the **existing fixtures**, not the code. If
the unit wants to keep these lines waived, the honest mechanism is "test-infrastructure
failure arm: the arm fires only if the test's own premise is false" — a different
sentence from "a test cannot afford the bound", and one that tells the next author
which fixture change would revive it.

### 3.3 `app.rs:7220,7221,7222` — the `#[cfg(not(test))]` twin (FELL)

The waiver says "no integration test can open the sandbox panel (it needs an
interactive session)". Integration tests link the library **without** `cfg(test)` —
that is exactly why `agent_sidecar_launch.rs` can drive the launch path at all — so
the real-platform body is in the target, and the panel needs no interactive session:

```rust
for ch in "/sandbox".chars() { app.handle_input(&ch.to_string()); }   // public API
app.handle_input("\r");
while let Ok(cmd) = op_rx.try_recv() { app.handle_cmd(cmd); }        // public API
```

Measured in the instrumented integration-target run (`lcov-integration-probe.info`):
**`7220 DA=1`, `7221 DA=1`, `7222 DA=1`** (the test-target line numbers verify the run
itself). The probe's own assertion is on `App::next_deadline` (`before=None`,
`after=Some`), because `do_render` paints only once `App::start` has set `running` and
`start` blocks on a reachable agent; a *painted-card* assertion needs a mock agent in
the integration scope (which does not exist yet) or a PTY against the built binary.
That is an assertion-strength caveat, not a reachability one: `/sandbox` opened the
card from the real-platform twin with no agent and no TTY.

*For the owner:* the lib-side equivalent assertion uses `top_overlay_text`
(crate-private); in a `tui/tests/*` target the observable is either
`App::next_deadline` + the emitted `UiCmd`s (what my probe uses) or a PTY against
`CARGO_BIN_EXE_future-tui` with a reachable agent.

### 3.4 `grpc_client.rs:1239` — `_ => Uncertain` for a `Transport` error (FELL)

The waiver enumerates only "a loopback peer that accepts and hangs up" (which lands in
the *other* `Uncertain` arm at 1241) and "the black-hole case". Four other addresses
reach the `_` arm of the `io::ErrorKind` match at 1239 — measured
`1239 DA=3` with:

```
future-rev-probe.invalid:9 -> uncertain     (name does not resolve)
bad addr with spaces       -> uncertain     (URI cannot be parsed)
127.0.0.1:99999            -> gone          (port range check → the 1237 arm)
[2001:db8::1]:9            -> uncertain     (non-routable v6)
```

The probe does not assert *which* arm ran (both return `Uncertain` by design); the
per-line counter is the evidence. The waiver's *category* is also wrong on its own
terms: no host or fixture is missing — a malformed `--grpc-addr` is a user input the
TUI already feeds to this function.

### 3.5 `grpc_client.rs:1344` — the reconnect poll's loop-top stop (FELL)

The waiver: *"reaching the loop top with the flag set needs a stop that lands without
an outstanding notify — a race the suite cannot produce deterministically"*. Two
facts undo it:

1. `disconnect()` is `stop.store(true)` **followed by `notify_waiters()`** (line 250),
   and `notify_waiters` retains no permit for a task that is not registered as a
   waiter — so a stop that lands while the poll is inside `probe_agent(...).await`
   (a 3 s connect window) or inside the backoff sleep **is** lost, and the loop top is
   the next thing that observes it. That is the race, and it is not exotic: the
   existing test stops the poll while it is probing.
2. The suite already uses the deterministic form of "a stop with no notify":
   `subscribe_loop_top_stop_check` does `client.inner.stop.store(true, SeqCst)` with
   the comment *"Stop flag set directly (no notify) → the loop-top check exits"* — for
   the *subscribe* loop. Pointing the same construction at the reconnect poll is a
   dozen lines (`wait_for_backoff_above` already exists to know when the poll has
   started probing).

Probe: after the stream is lost and the backoff has grown, store the flag with no
notify, then assert the backoff stops growing (an assertion on this line's behaviour:
the poll returned at the loop top instead of probing again). Measured **`1344 DA=1`**.
*Hazard:* because the wakeup is a lost-permit `notify_waiters`, this line can also flip
to `DA>0` by accident in a normal run — a coverage-oscillation risk in the same class
as the inventory's `feishu_ws.rs:254` (§5.4).

### 3.6 `terminal.rs:1283`, `grpc_client.rs:3483`, `grpc_client.rs:3828` — premise arms (FELL)

All three are "a test helper's own failure arm" and all three are reachable
deterministically while the suite stays green, with an assertion on the helper's
contract:

* `terminal.rs:1283` — the invariant is "no signal is delivered while it polls, because
  every test that raises one holds `crate::test_env::lock()`". Two holes. First, the
  lock serializes *other* tests; it does not stop the polling test from raising the
  signal itself.
  `libc::raise(SIGWINCH)` puts the byte in the self-pipe, the helper's very next
  `wait(0)` returns `Signal(28)` — an outcome its arm list does not enumerate — and the
  helper panics. Second, "no signal delivered" is not a suite-wide invariant at all:
  `terminal_posix.rs:926` (`signal_handler_writes_to_pipe_when_installed`) mutates the
  process-global `SIGNAL_PIPE_WRITE` (its lines 931/938) and invokes the handler **with
  no lock**, so a byte can land in whichever backend pipe is installed at that moment —
  the same arm, reached by race rather than by input (§5.5).
  Measured **`1283 DA=1`**, panic text `unexpected wait outcome: Signal(28)`.
  Worth recording: the helper's arm list disagrees with the documented reader protocol
  (`terminal.rs:351` handles `Signal(RESIZE_SIGNAL)`), so the assertion is *stricter*
  than production; if any SIGWINCH ever lands in the polling window the suite goes red.
* `grpc_client.rs:3483` — `attach` with nothing serving the address: 20 bounded
  attempts, then the panic. `#[should_panic("client never attached to a live event
  stream")]` is green and executes the line. Measured **`3483 DA=1`**.
* `grpc_client.rs:3828` — `wait_for_backoff_above(&client, u64::MAX)` returns `None`
  after 4 s; asserting `is_none()` is a caller-side assertion of exactly this arm.
  Measured **`3828 DA=1`**.

If the campaign's standard is that such arms are legitimately waived, the statement to
record is "the arm fires only if the caller's premise is false" — but for these three
the premise *can* be inverted by a green test, so under PLAN §4 they are gaps, not
unreachable lines.

## 4. Reasons corrected on rows that survive

Survival does not mean the record's sentences are right. Three need a different reason
or a narrower claim (REVIEW.md §7):

1. `terminal.rs:1283` — see above: the reason is a *timing* argument (and the arm list
   is stricter than the reader protocol), not "no signal is delivered".
2. `agent_supervisor.rs:141–143` — the reason given is "only reachable if the child
   cannot be waited on, which the OS does not do for a child this process spawned". The
   category should be `unreachable-by-construction`, not
   `unreachable-in-this-environment`: nothing about *this host* prevents it, the
   construction does.
3. `terminal.rs:1216,1217` — the category is defensible but the mechanism should name
   the two facts that matter: POSIX `Backend::new()` is infallible **and** the one-shot
   seam is spent by the preceding call in the same test. Asserting only the first would
   break if the seam stayed set.
4. `update.rs:104` — add the second reason the record misses: covering it needs a build
   whose display version is a published label, and such a build also fails the
   `index.rs` guard assertion (`this build has no published channel`), so the
   configuration change is not free.
5. `agent_supervisor.rs:267` — the first gate is `select_agent` ("no local fallback"),
   not the `cfg!(test)` early return; the `panic!` arm needs a *reachable* matching
   agent on port 1, which is what makes it unreachable.

## 5. Hazards this review found (for the flake units and the campaign total)

1. **The crate-scoped tui number is host-state dependent.** Which arm of the launch
   loop runs depends on what is resolvable as a candidate: the private target dir
   (measurement form) and `PATH` (host state). This host has
   `/opt/homebrew/bin/future` — `v0.0.2-8449acf6+local`, installed today at 17:10 —
   which the client *rejects on version* and which therefore supplies `126–129 DA=1`
   in the unit's own report. Delete that binary and the counters on 126–129 go to 0;
   put a matching one on `PATH` (or build `future` into the private target dir) and
   **121 flips to 1** inside the crate-scoped form too. My two runs are the two
   endpoints of that switch. Any before/after comparison of the tui number must state
   the candidates it could resolve.
2. **`cli_smoke::print_mode_without_message_exits_one` passes whether or not a sidecar
   starts.** With a launch it exits 1 from `index.rs:1130–1133` ("No prompt provided");
   without one it exits 1 from the `ensure_agent_running` error. The test asserts
   `Some(1)` either way, so the difference is invisible in the test result and visible
   only in the coverage counters — a silent coverage gate on an infrastructure
   accident. Worth an explicit assertion (`stderr` says which path was taken) if this
   line is to be relied on as covered.
3. **`terminal.rs`'s `FORCE_NEW_FAILURE` is a process-global one-shot.** The flag is
   stored at 1204 under `terminal_test_lock()` and consumed two instructions later at
   1205. I checked the discipline rather than assuming it: **all eleven test functions
   that construct a `Terminal` take that lock** (1148, 1176, 1202, 1314, 1344, 1670,
   1730, 1782, 1800, 1846, 1859), so the window is not open *today*; the risk is a
   future test that forgets the lock, and the comment at 1150 ("observed as a flake in
   CI") records the reverse symptom. If a consumer ever wins the two-instruction
   window, line **1206** fires and the suite goes **red**, not covered.
4. **`stop_notify` is `notify_waiters`** (§3.5): a lost wakeup can deliver
   `grpc_client.rs:1344` by accident, so this line can oscillate `0 ↔ 1` between runs
   of the *same* code. The author's §9.7(b) found no oscillation in two same-code runs;
   these runs do not rule it out, they only show the race did not land twice.
5. **A process-global signal pipe with one unlocked writer.** `SIGNAL_PIPE_WRITE`
   (`terminal_posix.rs:35`) is installed and cleared by `enable_raw`/`restore_raw`
   (209/230) *under the lock*, but `signal_handler_writes_to_pipe_when_installed`
   (926-943) stores and resets it **without any lock** and then calls the handler. If
   that interleaves with an enabled backend, a synthetic SIGWINCH byte lands in the
   polling backend's pipe → `terminal.rs:1283` panics → the suite goes red; its
   `store(-1)` at 938 can equally blank a concurrently enabled backend's signal path.
   That is a second, independent reason the 1283 waiver's "no signal is delivered"
   invariant does not hold. Mechanism only: I did not deliberately race it (the
   deterministic counterexample in §3.6 is what proves the line reachable).

6. **`agent_supervisor.rs:121` is a "report, don't claim" line in the campaign
   aggregate.** The workspace form counts it covered; the crate-scoped record waives
   it. Whichever the campaign counts, the two forms must not both be quoted as the
   same number (the author's §9.7(a) already flags the pairing; this review confirms
   the mechanism, adds the PATH dependency, and shows the covering test by name).

## 6. Reproducing this review

### 6.1 Reviewed sources (worktree, sha256)

```
f2bc65c27da2f307248a58757c4d3376c55f7b5cebf05db7b5bb7443008a7022  tui/src/agent_supervisor.rs
04350328d10967c80d066c9184df13f1bfaa97632e6ec732c666bc4153f874b5  tui/src/app.rs
d54e46793528f9a088d4187fd2a3f7b62ac7f7a178ec2f256f821a53ff16d45b  tui/src/terminal.rs
3b1d749998e5d332d9d741dcefcc1fc219e1fb801c73a72dc2ca4f5672510be0  tui/src/rpc/grpc_client.rs
33e853fba34e392971e5730c1ee26c0e04b94bb489ed9f970e4f670f1a72369a  tui/src/components/chat_area.rs
d73a9d656c215497f8da14f25cc5b058638e7c2d676b5b104b22c507454308a4  tui/src/update.rs
4f0f5b4858b4f080079e9770dc5f3778124ed33ed7ff946fe78780a0dfa006aa  tui/src/index.rs
8ef58182842b612e3d78ae81023b4179ad5f46748f9b5e7d7cad09285af0bf0f  docs/testing/module-tui.md
```

### 6.2 The two `agent_supervisor.rs:121` runs (no probe test needed)

```bash
cd /tmp/rev-tui                     # base 8449acf6 + the frozen tui/ overlaid
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/tmp/rev-tui-home CARGO_TARGET_DIR=/tmp/rev-tui/target/cov-tui-rev
ulimit -n 10240
cargo build -p future-cli           # the candidate the --workspace form builds for free
D=$CARGO_TARGET_DIR/llvm-cov-target/debug
install -m 755 target/debug/future $D/future                     # WITH a same-tree sibling
cargo llvm-cov -p future-tui --test cli_smoke --lcov \
  --output-path /tmp/rev-tui/probe/lcov-cli_smoke-with-sibling.info -j 8
rm -f $D/future                                                  # WITHOUT it (control)
cargo llvm-cov -p future-tui --test cli_smoke --lcov \
  --output-path /tmp/rev-tui/probe/lcov-cli_smoke-without-sibling.info -j 8
python3 /tmp/da.py /tmp/rev-tui/probe/lcov-cli_smoke-with-sibling.info \
        agent_supervisor.rs 105 150        # 121 → DA=1
python3 /tmp/da.py /tmp/rev-tui/probe/lcov-cli_smoke-without-sibling.info \
        agent_supervisor.rs 105 150        # 121 → DA=0, 107-109 → 2, 126-129 → 1
```

`/tmp/da.py` is the `DA:` extractor every counter above was read with — 20 lines, no
dependency beyond the stdlib, so the review's numbers can be re-derived without it:

```python
import sys, re
path, pat, lo, hi = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
cur = None; out = {}
for line in open(path, errors='replace'):
    if line.startswith('SF:'):
        cur = line[3:].strip()
    elif line.startswith('DA:') and cur and re.search(pat, cur):
        n, v = line[3:].strip().split(',')[:2]
        if lo <= int(n) <= hi:
            out.setdefault((cur, int(n)), v)
for (f, n), v in sorted(out.items()):
    print(f"{n}\tDA={v}\t{f}")
```

The headline check for the record itself:

```bash
python3 .future/cov100/covlines.py total  /tmp/tui-lcov-final6.info --under .worktrees/cov100-r2/tui/src
python3 .future/cov100/covlines.py detail /tmp/tui-lcov-final6.info --under .worktrees/cov100-r2/tui/src
```

### 6.3 Probe inventory (all inside the scratch tree; sha256 of what I ran)

| artefact | sha256 | what it probes |
|---|---|---|
| `probe/probe-app-snippet.rs` | `9af1124c027ef6fa929ef829ad67ee22c1c57450e0efc76e47b09b3841a9b654` | `app.rs:11995`, `app.rs:25296` |
| `probe/probe-grpc-snippet.rs` | `82534e9e22b69e46318edcec20b6fd49c6351aab3703e2d01500a73d2d83c4d9` | `grpc_client.rs:1239`, `1344`, `3483`, `3828` |
| `probe/probe-terminal-snippet.rs` | `af3db0954157585314a63054d03900113f876872d1139fecda3acf880a40cfc2` | `terminal.rs:1283` |
| `tui/tests/probe_rev_tui.rs` | `e7671f6dac35deb7855083f12f1e7090f493addf4ca4743525fa54561f867716` | `app.rs:7220`, `7221`, `7222` |
| `probe/apply-probes.py` | `d16baf7ecd6c2b910739f8ffcd5e499476a7963717a240845f6c662f851559c5` | inserts the snippets before the closing brace of each file's `mod tests` (idempotent, prints the patched hashes) |
| patched `tui/src/app.rs` | `77eefd3f06916f64d0646ac2eacbaada713a07655e01a0ff6a5115c6e770f53c` | |
| patched `tui/src/rpc/grpc_client.rs` | `a4a78ec87918fd6659f73b9b02088e4f7ce92fbdd411ab489540ce22c7e5eaad` | |
| patched `tui/src/terminal.rs` | `ba83307fb0e16f2361056a28a1cc6844276c0cdccd4e77b5f6cca0fe99d31114` | |

Probe reports (LCOV):

```
c2060917a5712257116a5459f10b341daa843d3374195c9e2161c840d5cfbbb1  lcov-lib-probes2.info          (all seven lib-row targets)
d6942bf9fac39109803512138c43681bdb3afa54b26ab8301d74a7f2a47d6c5a  lcov-lib-probes.info           (first probe set)
95803bc19c7ffb8f8dffc9984151dbdb1abed88836da97e11eab7793af6aef25  lcov-integration-probe.info    (7220-7222)
8937bb4434fda9e97ce58d5caf5dd554aad22778164ba42a0329bab62f5f3273  lcov-cli_smoke-with-sibling.info
de1315d58f6749e6b0f2bf31c0d84c1c201022955651fa79afa578294e9aa621  lcov-cli_smoke-without-sibling.info
```

Run them again with:

```bash
cd /tmp/rev-tui
python3 probe/apply-probes.py                       # idempotent; re-hash the sources
cargo llvm-cov -p future-tui --lib --lcov \
  --output-path /tmp/rev-tui/probe/lcov-lib-probes2.info -- probe_          # 7 lib rows
cargo llvm-cov -p future-tui --test probe_rev_tui --lcov \
  --output-path /tmp/rev-tui/probe/lcov-integration-probe.info               # 3 app rows
# plain (uninstrumented) run of the same probes, which also prints the reached
# panic texts and the probe_agent classification:
cargo test -p future-tui --lib -- probe_ --nocapture --test-threads=4
```

Expected: `18 passed`, the three `should_panic` probes pass with
`the history page never settled` / `unexpected wait outcome: Signal(28)` /
`client never attached to a live event stream`, and `probe_agent` prints
`[…] -> uncertain` for the name/parse/v6 addresses. **Control lines stayed `DA=0`
across these runs** (`terminal.rs:1206/1216/1217`, `chat_area.rs:2651`,
`update.rs:104`, `agent_supervisor.rs:141–143`), so the probe set is specific
rather than a shotgun.

The scratch tree (`/tmp/rev-tui`, 8.1 GB including its instrumented `target/`) stays
alive until this review is accepted; delete it then, and note that deleting it does
not touch the campaign worktree or its `target/`.

## 7. What I could not check

* **I did not re-run the full `future-tui` suite** (lib + the four integration
  targets) or re-measure the crate myself: with eight workers on this host at load
  19–40, a full instrumented run is the expensive artefact the unit already owns. I
  verified its *artefact* (`tui-lcov-final6.info`) and the per-line list, and used
  filtered instrumented runs (`--test cli_smoke`, `--lib -- probe_`, `--test
  probe_rev_tui`) for the counterexamples. So I can say the 24 lines match the cited
  report; I cannot independently re-derive the 99.9601 % end-to-end.
* **Windows rows are argued from source, not measured** — macOS is my host:
  `terminal.rs:1216/1217` and `1276/1277/1280` would be measured on Windows, and I did
  not verify that Windows actually produces the `Err`/`Resize` outcomes they claim
  (their side of the argument is plausible but unproven here).
* **`terminal.rs:1283`'s suite-wide invariant** ("every test that raises SIGWINCH holds
  `test_env::lock()`") I checked against the raisers and global-pipe writers in both
  files, which is how §5.5's unlocked writer surfaced. I did not deliberately reproduce
  the red suite that race would cause.
* **I did not build the mock agent / PTY fixture** that a painted-card assertion for
  `app.rs:7220-7222` would want (see §3.3); the line's reachability does not depend on
  it.
* **`update.rs:104`** — I did not attempt a published-channel build
  (`FUTURE_VERSION=1.2.3`) plus a real fetch; the row's survival rests on the code path
  (`manifest_url` gate) and the network/root requirement, plus my read of the two
  manifest call sites. If the supervisor wants that row attacked harder, the recipe is
  `FUTURE_VERSION=1.2.3 cargo llvm-cov -p future-tui --lib` and a call to
  `update::check("1.2.3")` — but the sibling guard test in `index.rs` would then fail,
  so it is a *build-configuration* experiment, not a test addition.
* **`agent_supervisor.rs:121` in the unit's own crate-scoped form** — I showed it is
  reachable in the campaign's environment (workspace form) and constructed the input
  that flips it in the crate-scoped form (a same-tree sibling candidate). I did **not**
  write the mock-agent-on-the-local-endpoint fixture that would cover it without
  relying on the workspace build; §3.1 gives the shape and names the missing piece
  (no public readiness-server helper in `future_rpc`).
* The unit's other claims (§9.1–§9.3 harness story, §9.6 repro block, §9.7(c)
  flake-hazard list, the `tmux-diff.sh` 57/57 rendering gate) were not re-run.

## 8. Rework implied (for the supervisor to allocate)

1. **`agent_supervisor.rs:121`** — remove from the ledger (covered in the workspace
   form; my §6.2 run is the reproduction) or cover it in the crate-scoped form with a
   local-endpoint readiness fixture. This changes the campaign total either way.
2. **Coverable now, cheap** (all ten remaining falling lines): `app.rs:11995`
   (paused clock), `app.rs:25296` (20 s, asserted), `app.rs:7220–7222` (integration
   test, §3.3), `grpc_client.rs:1239` (malformed/unresolvable address),
   `grpc_client.rs:1344` (no-notify stop; **also fix the lost-wakeup** or state it),
   `grpc_client.rs:3483`, `grpc_client.rs:3828`, `terminal.rs:1283`
   (and decide whether the helper's arm list should match the reader protocol).
3. **Reason corrections without rework**: §4 items 2–5.
4. **Hand to the flake units**: §5.1 (host-state dependence of the crate-scoped tui
   number), §5.2 (the `cli_smoke` exit-code coincidence), §5.3
   (`FORCE_NEW_FAILURE` cross-test consumption → a red-suite flake, discipline is
   currently intact), §5.4 (`stop_notify` `notify_waiters` lost wakeup → `1344` can
   oscillate), §5.5 (`SIGNAL_PIPE_WRITE` mutated with no lock → `1283` can fire by race
   and the suite goes red).
