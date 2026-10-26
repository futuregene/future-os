# Two installers, one Agent channel runtime (2026-10-26)

A one-off record, not a design document: the reproduction, the root cause with
line numbers, the fix, and what the tests do and do not cover. Everything below
was measured on this machine; the commands are written out so each row can be
re-checked.

## 0. Summary

| | |
|---|---|
| State | every entry registered the process-wide Agent channel driver runtime by `set`ting a `OnceLock` *and* wrapping the "already configured" error in `expect`, inside its own `Once` |
| Effect | a second installer in the same process **panicked**; because the panic happened inside its `Once`, that `Once` was then poisoned and every later install in the process failed for a reason unrelated to the first. One run of the Desktop suite turned this into **38 failing tests** out of 364 |
| Root cause | `configure_channel_runtime` (`host/src/agent_client.rs:224`) reported "already configured" as `Err`; three callers — the Desktop entry (`desktop/src-tauri/src/agent_bridge/client.rs:10`), the CLI entry (`cli/src/headless/mod.rs:841`) and the shared test fixture (`host/src/test_support/agent.rs:336`) — each `expect`ed it inside a `Once`/`OnceLock` of their own |
| Fix | one idempotent install port in `future-host` with two outcomes, both successful (`Installed` / `AlreadyInstalled`); the `OnceLock` is the only arbiter; the winner parks its runtime forever, the loser shuts its own down. No caller `expect`s an install outcome any more |
| Covered by | 3 new tests (2 in `future-host`, 1 in the Desktop entry) |
| Not covered | the concurrent race itself — the tests pin the install *contract*, not thread scheduling (see §5.3) |

The channel-runtime failure is **not** what made CI run
[40262600990](https://github.com/futuregene/future-os/actions/runs/40262600990)
red. That run failed on a clippy argument-count lint (fixed by `72950dd26`, which
landed after the frozen PR head) and on a pty exhaustion in the macOS runner
(§7). This record exists because the poisoning class makes *every* future red
run untrustworthy, not because it explains that one.

## 1. Reproduction

The bug needs **two installers in one process**. That is the ordinary shape of a
test binary: `desktop/src-tauri`'s dev-dependencies enable `future-host`'s
`test-support` feature (`desktop/src-tauri/Cargo.toml:45`), so the fixture that
`agent_bridge::test_support::mock_agent` installs (`host/src/test_support/agent.rs`)
and the Desktop entry's own install run in the same address space. Whichever
runs first wins the `set`; the other panics.

```bash
# pre-fix, on the branch base ed012b3a9
ulimit -n 10240                       # this machine's soft fd limit is 256
cargo test --locked -p futureos --lib
```

| | before the fix (`ed012b3a9`) | after (this branch) |
|---|---|---|
| runs of `cargo test -p futureos --lib` | 5 | 9 |
| runs red | **3** | 2 (both for an unrelated reason, §1.2) |
| failures in a red run | 38 | — |
| runs with a channel-runtime panic or a poisoned `Once` | **3 of 5** | **0 of 9** |
| green runs | 2 × 364 passed | 7 × 365 passed |

The primary panic, and the cascade it caused (one red run, verbatim):

```
thread 'commands::agent::tests::get_agent_status_forwards_the_agents_phase' panicked at
  desktop/src-tauri/src/agent_bridge/client.rs:10:14:
  entry configures the Agent channel runtime once: Message("Agent channel runtime already configured")

thread 'commands::agent::tests::agent_status_requires_the_running_agent_version_to_match' panicked at
  desktop/src-tauri/src/agent_bridge/client.rs:8:17:
  Once instance has previously been poisoned
...
test result: FAILED. 326 passed; 38 failed; 0 ignored; 0 measured; 0 filtered out
```

37 of the 38 carried `Once instance has previously been poisoned` at
`client.rs:8` — the *first* panic poisons the `Once`, and every later entry
install in the process then fails on the poison rather than on anything real.

### 1.1 Why CI never showed it

`gh api .../actions/jobs/<desktop job>/logs` for the eight most recent
`Rust macOS test / desktop` jobs contains **no** `previously been poisoned` and
**no** `Agent channel runtime already configured`. The GitHub runner exposes
3–4 cores; this machine exposes 32, so many more test threads start at once and
the two installers are far more likely to collide. The defect is a scheduling
race that a fast machine makes easy to hit — not a Desktop-only, CI-only or
machine-specific behaviour.

### 1.2 The two red runs after the fix are a different, load-related failure

Both were inspected for the fix's signatures: `0` occurrences of `poisoned` and
`0` of `already configured` in either. One of them was captured in full:

```
test skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin ... FAILED
  ... CLI discovery timed out. Install a matching CLI beside Desktop or correct PATH;
      no fallback CLI was executed.; skipping built-in skill bootstrap
  panicked at desktop/src-tauri/src/skills_bootstrap.rs:229:
  the spawned CLI must have run its `init`
```

That is the 3-second CLI-discovery budget expiring on a loaded machine (the red
runs took 98 s and 104 s against 64–67 s for the green ones), in a subsystem
this change does not touch. Names of the other run's four failures were not
captured; its signature counts were zero.

`cargo test -p future-cli --lib` has a flake of the same family: 1398 of 1399
twice, always
`commands::mcp_client::tests::process::the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest`,
with the fake MCP server (`/bin/sh -c`, piped into `sed`) reporting
`/bin/sh: line 8: sed: command not found` — i.e. the child's inherited
environment lost `PATH` entries. Run alone with the same `PATH` it passes 4/4,
so the trigger is the concurrent `std::env::set_var` calls other suite tests make
(`cli/src/test_env.rs` `EnvGuard`) while this child is being spawned: `set_var`
mutates the process `environ` that `spawn` copies, which is exactly why it is
unsafe in Rust 2024. Pre-existing, no process is spawned by this change, and not
fixed here.

## 2. Root cause

The registry is one process-wide slot:

```rust
static CHANNEL_RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

pub fn configure_channel_runtime(handle: tokio::runtime::Handle) -> Result<(), crate::Error> {
    CHANNEL_RUNTIME
        .set(handle)
        .map_err(|_| "Agent channel runtime already configured".into())
}
```

`OnceLock::set` is a fine arbiter: it is atomic and exactly one caller wins. The
defect is what the loser was told and what three callers did with it:

| installer | pre-fix code | what a losing install did |
|---|---|---|
| Desktop entry | `client.rs:6-11` `static INITIALIZED: std::sync::Once` → `configure_channel_runtime(agent_channel_runtime()).expect("entry configures the Agent channel runtime once")` | **panicked inside `Once::call_once`** → the `Once` is poisoned for the rest of the process |
| CLI entry | `cli/src/headless/mod.rs:838-843`, same shape | same |
| shared test fixture | `host/src/test_support/agent.rs:325-344`: `INSTALLED: OnceLock<()>`, `if agent_channel_runtime().is_ok() { return; }` then `.expect("fixture installs its channel runtime once")` | panicked inside `OnceLock::get_or_init` |

Two design faults, both fixed:

1. **"Already installed" was an error.** It is the ordinary outcome of a second
   installer, and reporting it as `Err` invites exactly the `expect` above.
2. **Each installer kept a second, private state machine** (`Once` /
   `OnceLock<()>`) in front of the arbiter. Two state machines over one slot can
   disagree — here, "my `Once` has not run yet" while the slot is already full —
   and the disagreement turned into a panic. The registry alone is enough.

The `test_support` fixture even *had* the right guard (`if
agent_channel_runtime().is_ok() { return; }`) and still lost: the guard is not
atomic with the `set`, so a Desktop install landing in between panics the
fixture.

## 3. The fix

One port, one arbiter, three callers with the same shape. Nothing about *who
provides a runtime in production* changes: `future-host` still refuses to
construct a fallback, and `agent_channel_runtime()` is still `pub(crate)` and
still returns an error when no entry installed one
(`host/src/agent_client.rs`, "Entry must configure the Agent channel
runtime…"). The `docs/architecture/headless-package-development.md:388`
invariant stands.

### 3.1 The port (`host/src/agent_client.rs`)

```rust
pub enum RuntimeInstall { Installed, AlreadyInstalled }        // both are successes

pub fn install_channel_runtime(handle: tokio::runtime::Handle) -> RuntimeInstall {
    install_into(&CHANNEL_RUNTIME, handle)                     // set() is the arbiter
}

pub fn channel_runtime_installed() -> bool { ... }

/// The shape every installer uses: build only when nobody registered yet, and
/// keep the runtime running only when this call won.
pub fn provide_channel_runtime(
    driver_thread: &str,
    build: impl FnOnce() -> tokio::runtime::Runtime,
) -> RuntimeInstall {
    if channel_runtime_installed() { return RuntimeInstall::AlreadyInstalled; }
    let runtime = build();
    match install_channel_runtime(runtime.handle().clone()) {
        RuntimeInstall::Installed => { /* park a thread on block_on(pending()) */ }
        loser => { runtime.shutdown_background(); loser }
    }
}
```

- **No failure outcome.** The type has two variants and no `Err`; a caller cannot
  wrap the ordinary case in `expect` because there is nothing to unwrap.
- **`OnceLock` is the sole arbiter.** Chosen over keeping a `Once` in front of
  it: the failure mode that produced this record was *two* state machines
  disagreeing, so a second one is exactly what must not come back.
  `OnceLock::set` is already atomic and idempotent, and `provide_channel_runtime`
  makes the repeated call cheap by checking `channel_runtime_installed()` first —
  a losing call does not even build a runtime (pinned by a test, §5.2).
- **The keep/discard pairing lives in one place.** The winner must park its
  runtime forever (`block_on(pending())`), and the loser must shut its own down —
  a winner that lets its runtime drop kills the shared channel's h2 driver task,
  and the loser cannot simply drop the runtime it built, because
  `install_channel_runtime` is reachable from async code (Desktop's
  `get_agent_readiness`), where dropping a multi-thread runtime panics. The
  discard therefore goes through `Runtime::shutdown_background`, which tokio
  documents as safe "if you want to drop a runtime from within another runtime".
  Both halves are in `provide_channel_runtime`, so no entry has to get the
  pairing right by hand — the invariant is stated in the port's doc comment,
  where it is the whole reason the design holds.

### 3.2 The three callers

| file | after |
|---|---|
| `desktop/src-tauri/src/agent_bridge/client.rs` | `install_channel_runtime() -> RuntimeInstall` is `provide_channel_runtime("agent-channel-runtime", build)`; the `Once` is gone, its valuable doc comment (why the runtime must never shut down) now sits on the builder closure. Six call sites in the crate are unchanged — they ignore the outcome |
| `cli/src/headless/mod.rs` | same shape, `agent_channel_driver()` returns the runtime instead of a handle |
| `host/src/test_support/agent.rs` | `provide_channel_runtime("fixture-agent-channel", build)`; its `INSTALLED: OnceLock<()>` and its hand-written guard are gone |

Behaviour that does **not** change: exactly one driver runtime per process, on a
thread that never returns, whichever installer wins; a losing installer now
returns a value instead of panicking.

## 4. Measured before vs after

| | before | after |
|---|---|---|
| second install in one process | panic (`Agent channel runtime already configured`) inside the caller's `Once`, then poison | returns `AlreadyInstalled`, nothing changes |
| a later install after that | panics on the poisoned `Once` (`Once instance has previously been poisoned`) | returns `AlreadyInstalled` |
| `cargo test -p futureos --lib`, 32-core machine, 9–5 runs | 3 of 5 red, 38 failures each | 0 of 9 with a channel-runtime signature; 7 green × 365, 2 red on §1.2 |
| `cargo test -p future-host --lib` | — | 1828 passed, 0 failed, 10 ignored (468 s) |

## 5. Tests

| test | where | what it pins |
|---|---|---|
| `the_first_install_wins_and_the_second_reports_already_installed` | `host/src/agent_client.rs` (unit, private slot) | `Installed` then `AlreadyInstalled`, no panic, and the slot is populated — the outcome replaces the old `Err` |
| `a_losing_installer_neither_builds_nor_parks_a_runtime` | same | the losing call does **not** run the build closure (it panics if reached): "already installed" means *do nothing*, not "build and throw it away" |
| `a_second_installer_is_a_normal_outcome_not_a_panic` | `desktop/src-tauri/src/agent_bridge/client.rs` (unit, real process slot) | another installer registers first, then the **entry installs twice**: both are `AlreadyInstalled` and neither panics. Reaching the end of the test is the no-panic assertion |

The two `host` tests use a private `OnceLock` of their own
(`install_into`/`provide_into` take the slot as a parameter) so they are
deterministic regardless of what other tests in the shared binary have already
installed. The Desktop test deliberately uses the **real** process-wide slot,
because that is the state the defect lived in.

### 5.1 Falsified, not just green

Both tests were run against the pre-fix semantics (the `set` failure restored to
`panic!("Agent channel runtime already configured")`, and the early "already
installed" return removed, i.e. what the old `Err` + `expect` pair did):

| test | pre-fix semantics | with the fix |
|---|---|---|
| `the_first_install_wins_and_the_second_reports_already_installed` | **FAILED** — `panicked at host/src/agent_client.rs:294: Agent channel runtime already configured` | ok |
| `a_losing_installer_neither_builds_nor_parks_a_runtime` | **FAILED** — same panic | ok |
| `a_second_installer_is_a_normal_outcome_not_a_panic` | **FAILED** — `panicked at host/src/agent_client.rs:294: Agent channel runtime already configured` | ok |

The falsification edit was removed afterwards and the file verified byte-identical
(`shasum -a 256 -c`), so the recorded numbers are from the fixed source.

### 5.2 What the tests do not cover

- **Only the install contract, not the thread-scheduling race.** No test starts
  two installers concurrently and asserts on the interleaving: in one process the
  winner is decided by `OnceLock::set` and the test cannot force a losing
  *schedule*. Two callers in sequence is the strongest deterministic statement —
  and the pre-fix full-suite runs (§1) show that this sequential shape is what
  actually fires in practice: the fixture and the entry race, not two threads.
  What the two-thread race would additionally exercise is the loser's
  `shutdown_background` discard, which no test reaches on this host.
- **The Desktop entry's discard path.** Same reason: for the entry to lose, a
  fixture install must land between `channel_runtime_installed()` and
  `OnceLock::set`. Not reproducible on demand.
- **`future-cli`'s entry.** It has the same three-line shape as the Desktop
  entry and no test installs into it; only compilation and clippy cover it.

## 6. Files

| file | change |
|---|---|
| `host/src/agent_client.rs` | `configure_channel_runtime` → `install_channel_runtime` (`RuntimeInstall`), new `channel_runtime_installed` / `provide_channel_runtime`, two unit tests |
| `desktop/src-tauri/src/agent_bridge/client.rs` | `Once` removed; install is `provide_channel_runtime`; one unit test |
| `cli/src/headless/mod.rs` | `Once` removed; install is `provide_channel_runtime` |
| `host/src/test_support/agent.rs` | `INSTALLED: OnceLock<()>` and the manual guard removed |

## 7. The macOS `openpty: Device not configured` from the same CI run

**Not the same cause**, and not fixed here. `Rust macOS test / desktop` job
[121378735419](https://github.com/futuregene/future-os/actions/runs/40262600990/job/121378735419)
failed one test,
`terminal::manager::tests::global_capacity_is_enforced_across_conversations`,
while *creating* its 33rd PTY:

```
create session: Spawn("SPAWN_FAILED: openpty: failed to openpty:
  Os { code: 6, kind: Uncategorized, message: \"Device not configured\" }")
```

errno 6 is `ENXIO`, and `openpty(3)` on macOS documents it as
`[ENXIO] The sysctl(3) limit "kern.tty.ptmx_max" number of pseudo-terminal
devices was reached.` — a host-wide PTY limit, not this code's logic:

- the test holds 32 live PTYs on purpose (`MAX_SESSIONS = 32`) and asserts that
  the 33rd is refused with 429; it died in the *setup* of that 33rd session, so
  it never reached its own assertion;
- every neighbouring PTY test in the same binary passed, and the same test never
  failed in 14 other recent `Rust macOS test / desktop` jobs, nor in the
  Linux/Windows desktop jobs;
- on this machine the count peaked at 85 `kern.tty.ptmx_max`-counted ptys during
  the terminal-heavy stretch of the suite and returned to baseline — no leak and
  no monotonic growth across the run (sampled every 300 ms);
- the same test passed 3/3 in isolation and 9/9 as part of the full suite on this
  branch.

So it is a burst of concurrent PTYs on a runner whose `kern.tty.ptmx_max` is
smaller relative to its core count, i.e. a capacity flake on the runner side. Fixing
it properly needs the runner's limit (unknown from here) or an explicit
serialisation of the PTY-heavy tests, and neither is a change this record's task
covers; a rerun is the correct response. It is listed here so the two failures
from that CI run are not conflated.

## 8. Environment

| | |
|---|---|
| Machine | macOS 26.6.2 (25G83), Apple silicon, 32 cores |
| Base commit | `ed012b3a9` on `origin/dev-huichen`; branch `fix/agent-runtime-idempotent` |
| Toolchain | pinned `1.99.1` (`rust-toolchain.toml`), installed through `rustup`; `Homebrew`'s `rustc` (1.96.0) kept out of `PATH` |
| Test env | `FUTURE_VERSION=0.0.2-ci-record`, `ulimit -n 10240` (this machine's soft limit is 256, which breaks unrelated fd-heavy tests) |
| Checks | `cargo fmt -p future-host -p future-cli -p futureos -- --check`, `cargo clippy -p future-host --all-targets -- -D warnings`, `cargo clippy -p future-cli --all-targets -- -D warnings`, `cargo clippy -p futureos --all-targets -- -D warnings`, `cargo test -p future-host --lib`, `cargo test -p futureos --lib` ×9, `cargo test -p future-cli --lib` |
