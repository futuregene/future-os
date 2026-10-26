# A flaky-test audit, and the two that were still live (2026-10-26)

A one-off record, not a design document. The task was "fix all the flaky tests";
this is what that resolved to — every documented flake checked against the code
rather than against its note, the two that still reproduced fixed at the root,
and the ones deliberately left alone with the reason.

## 0. Summary

| | |
|---|---|
| Starting point | 13 flakes recorded as open across `.future/memory/testing-gotchas.md` and `docs/testing/flaky-*.md` (6 315 lines) |
| Already fixed by earlier work | **5** — the note was stale, the code already carried the fix (and in two cases an enforcing test) |
| Still live, fixed here | **2** — `mcp_client`'s fake MCP servers, and the Desktop bootstrap's CLI probe |
| Not reproducible here | **1** — the `future-rpc` endpoint-lease race (12/12 green) |
| Environmental, not a test defect | **2** — macOS PTY exhaustion, `database is locked` on a loaded Linux runner |
| CI-only, no local reproduction | **3** — recorded as unchanged |

The two live ones share a shape: **a test decided by something outside the code
under test** — the inherited process environment in one case, a wall clock in the
other. Neither was fixed by loosening an assertion; both were fixed by removing
the dependency.

## 1. The audit, as measured

"Documented as unfixed" is not evidence that a flake is still there. Each entry
below was checked in the tree, not in its note.

| recorded flake | verdict on this tip | evidence |
|---|---|---|
| Desktop HOME↔MOCK lock inversion (total deadlock in the lib suite) | **fixed 2026-10-17** | `host/src/remote/test_support.rs:430` documents the order; `settings.rs:420` takes MOCK before HOME; `no_body_takes_the_home_guard_before_the_mock_lock` (line 2162) scans all 191 `src/` files and fails on any inverted body, with its own control test |
| `cli/tests/bin.rs` agent-singleton-lock race | **fixed** | every spawned agent gets its own `HOME` (`cli/tests/bin.rs:20-30`, `tempdir()` at 63/85/128/143/147/200), which is the "根治方向" the note named |
| mobile `useSkillRecommendation` "already shown today" (local midnight) | **fixed** | the test imports `today()` (`skillRecoBudget.ts:32`) which builds the *local* calendar day from `getFullYear/getMonth/getDate`; the note's defect (`toISOString().slice(0,10)` = UTC day) is gone and the test carries a comment naming it |
| `agent/tests/cli_smoke.rs` profile-timer vs bind-log race | **fixed by removing the race** | `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint` no longer asserts a log line ordering; it asserts the `~/.future/run` directory and its `0o700` mode, which the timer cannot overtake |
| `safari_manager::start_launch_success_against_fake_driver` 10 s window | **widened** | ready deadline is now 20 s with a 30 s outer timeout (`safari_manager.rs:930-936`) |
| `future-rpc` local-endpoint lease (`AddrInUse`) | **not reproducing** | 12/12 `cargo test -p future-rpc --lib` green on this 32-core host (the note measured 9/30 and 16/30) |
| macOS `openpty: Device not configured` | **environmental** | `ENXIO` from `kern.tty.ptmx_max`; the test dies creating its 33rd PTY before reaching its own assertion. 1 failure in 15 recent macOS desktop jobs; 9/9 green here |
| `DatabaseBusy: database is locked` (Linux CI) | **environmental** | fails in `TestHome` construction, i.e. the fixture, not the test's subject |
| desktop backend `chunk_pull` 500 ms budget | **CI-only, unchanged** | no local reproduction; the note's advice (rerun) still stands |

## 2. Live flake 1 — the fake MCP servers needed `sed` from a `PATH` other tests were rewriting

### 2.1 Symptom

Two full-suite runs of `cargo test -p future-cli --lib` in a row (32-core host)
failed 1398/1399, always the same test:

```
---- commands::mcp_client::tests::process::the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest
panicked at cli/src/commands/mcp_client.rs:1032:18:
the handshake is a normal one: "the MCP server 'chatty' failed its handshake:
  the MCP server sent an unreadable frame while answering 'initialize':
  expected value at line 1 column 23
--- server stderr (last lines) ---
stderr-07
...
/bin/sh: line 8: sed: command not found"
```

The test passed 4/4 when run alone. The same construct appeared in the
`a_poisoned_stderr_tail_is_tolerated_by_the_drain_and_the_reader` arm.

### 2.2 Root cause

The fake server is a `/bin/sh -c` script, and it parsed the request id by piping
the frame through `sed`:

```sh
id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
```

`sed` is resolved through the **inherited** `PATH`. `ServerProcess::start`
(`cli/src/commands/mcp_client.rs:350`) spawns without `env_clear`, so the child
gets whatever copy of `environ` the spawn makes — and `std::env::set_var` is
`unsafe` in Rust 2024 precisely because that copy races with it. libtest runs the
binary's tests in parallel, several of them replace `HOME`/`PATH`/`FUTURE_HOME`
(`crate::test_env::EnvGuard`), and the `mcp_client::tests::process` module takes
no `lock_env()` at all (0 occurrences). A torn copy that lost `/usr/bin` leaves
`sed` unresolvable; the id then expands to empty, the reply becomes
`{"jsonrpc":"2.0","id":,"result":{}}`, and the client rejects it as an unreadable
frame — which is exactly the `expected value at line 1 column 23` in the log.

### 2.3 Fix

Every fake server in the crate parses the id (and one, a path) with shell
parameter expansion instead, so it needs **no external binary at all**:

```sh
id=${line#*\"id\":}
id=${id%%,*}
```

Ten sites: 1 + 1 in `cli/src/commands/mcp_client.rs`, 8 (+1 string `path`) in
`cli/src/commands/mcp_servers.rs`. `grep -rn "sed -n" cli/src` now returns nothing.

### 2.4 Falsified

The dependency is provable directly, with the servers' own scripts and a `PATH`
that has no `/usr/bin` in it (the torn-copy state):

| script | `PATH=<empty dir>` |
|---|---|
| old (`sed -n …`) | `/bin/sh: sed: command not found` → `id=[]` → the reply is not valid JSON |
| new (parameter expansion) | `id=[3]` — correct, with no `PATH` at all |

(The control matters: with `env -i`, i.e. `PATH` *unset*, the old script still
works, because `sh` then falls back to a compiled-in default `PATH`. Only a `PATH`
that is set and wrong reproduces the flake — which is what a torn `environ` gives
you, and why the failure never appeared when run alone.)

| | before | after |
|---|---|---|
| full `cargo test -p future-cli --lib` | 2 of 2 runs red, 1398/1399 | **3 of 3 runs green, 1399/1399** |
| `-- commands::mcp_` (81 tests) | — | 5 of 5 runs green |

## 3. Live flake 2 — the Desktop bootstrap was decided by the 3 s probe clock

### 3.1 Symptom

A full-suite run of `cargo test -p futureos --lib` on a loaded 32-core host
(98 s and 104 s against 64–67 s for the green runs) failed three tests, all in
`skills_bootstrap`, all reporting the same cause:

```
isolated skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin failed:
  FutureOS: CLI .../bin/future: CLI discovery timed out. Install a matching CLI
    beside Desktop or correct PATH; no fallback CLI was executed.; skipping built-in skill bootstrap
  the spawned CLI must have run its `init` (drain loop exit ".../init-ran")
```

### 3.2 Root cause

The probe that discovers the CLI kills its child after `PROBE_DEADLINE`
(3 s, `host/src/cli_tool.rs:180`). Every bootstrap test sets `PATH` to a fixture
directory and then waits for the fixture shell script to answer
`future version --json` — so the test's outcome is a race between a busy host's
scheduler and a wall clock, not a property of the bootstrap.

The crate already had the anti-race seam, and its own doc comment argues the case
exactly ("with a deadline in play those assertions are decided by a race between a
busy host's scheduler and this clock, not by the code under test"). But it was
gated **`#[cfg(test)]`**, and Desktop compiles `future-host` as a *dependency* of
its own test binary, where `cfg(test)` is false — so the entry tests could not
reach the seam and were the only ones left racing.

### 3.3 Fix

The seam now has the same gate as the rest of the crate's fixtures —
`#[cfg(any(test, feature = "test-support"))]` — and `ProbeDeadlineGuard` is public
with `without_deadline()`. The two Desktop bodies that assert *what discovery
captured* arm it:

| site | why it needs it |
|---|---|
| `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin` | the assertion is that the discovered CLI ran `init` |
| `skills_bootstrap::block_fixtures::host_fixture` | all three cases (vanished interpreter, stop kills a live child, unconfirmed exit) first need discovery to succeed |

The arm that asserts the **kill** — `a_silent_cli_is_killed_and_reported_as_a_timeout`
— keeps the production budget, which is what pins the constant to 3 s. Production
behaviour is unchanged: without `test-support` the seam does not exist.

It stays thread-local, deliberately: libtest runs each test on its own thread, and
the bootstrap body runs in a re-exec'd child, so arming it inside the body cannot
reach another test's probe (`docs/testing/flaky-tauri.md` item 27 is the leak
class this avoids).

### 3.4 Falsified

Arming a 1 ns deadline instead of no deadline reproduces the original CI failure
**verbatim** — the same two lines, from the same assertion:

```
test skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin ... FAILED
  FutureOS: CLI .../bin/future: CLI discovery timed out. ...; skipping built-in skill bootstrap
  the spawned CLI must have run its `init` (drain loop exit ".../init-ran")
```

and `without_deadline()` makes it green. That is the proof the test was decided by
the clock and that the change removes the dependency rather than hiding it.

| | before | after |
|---|---|---|
| full `cargo test -p futureos --lib` | 2 red of 6 runs, 3–4 `skills_bootstrap` failures each | **3 of 3 green, 365/365** |
| `-- skills_bootstrap::` (12 tests) | — | 3 of 3 green |

## 4. What was deliberately not changed

- **macOS `openpty: Device not configured`** (§1). `ENXIO` is
  `kern.tty.ptmx_max`, a host-wide limit: the test holds 32 PTYs on purpose and
  died creating the 33rd, before its own assertion. It failed 1 of 15 recent
  macOS desktop jobs and 9/9 here; a fix needs the runner's limit or serialising
  the PTY-heavy tests, and neither is justified by evidence available here.
- **`future-rpc` endpoint lease.** 12/12 green on this host, so there is nothing
  to fix against — changing it would be guessing. The note's measurement stays
  valid as history.
- **`database is locked`, the 500 ms chunk-pull budget.** CI-only; no local
  reproduction, so any change would be speculative.
- **`mcp_servers` locking.** Its tests take `lock_env()` consistently among
  themselves; the script change makes them immune to the `PATH` half of the
  hazard, and a full env-serialisation refactor is not warranted without a
  reproduction.

### 4.1 Boundary worth stating

Making the fake servers PATH-free fixes *those* tests. It does not fix the
general hazard that the CLI test binary mutates process-global environment
concurrently with spawns — `std::env::set_var` is `unsafe` in Rust 2024 for
exactly this reason, and `mcp_client::tests::process` still takes no `lock_env()`.
A test there that spawns something needing an inherited variable would be exposed
again. The durable fix is to stop mutating the environment from tests (pass it in
the spec, as `ServerSpec.env` already allows) or to serialise every spawn behind
`lock_env()`; both are larger than this record.

## 5. Files

| file | change |
|---|---|
| `cli/src/commands/mcp_client.rs` | two fake servers parse the id with parameter expansion, no `sed` |
| `cli/src/commands/mcp_servers.rs` | eight fake servers likewise, plus the `path` extraction |
| `host/src/cli_tool.rs` | probe-deadline seam available under `test-support`; `ProbeDeadlineGuard` public with `without_deadline()` |
| `desktop/src-tauri/src/skills_bootstrap.rs` | the two capture-asserting bodies arm the seam |

## 6. Environment

| | |
|---|---|
| Machine | macOS 26.6.2 (25G83), Apple silicon, 32 cores |
| Base commit | `f7ed9bb84` on `origin/dev-huichen`; branch `fix/flaky-tests` |
| Toolchain | pinned `1.99.1` via `rustup` (Homebrew's 1.96 kept off `PATH`) |
| Test env | `FUTURE_VERSION=0.0.2-flaky`, `ulimit -n 10240` |
| Checks | `cargo fmt -p future-host -p future-cli -p futureos -- --check`; `cargo clippy --locked -p future-host -p future-cli -p futureos --all-targets -- -D warnings`; `cargo test -p future-cli --lib` ×3; `cargo test -p futureos --lib` ×3; `cargo test -p future-rpc --lib` ×12 |
