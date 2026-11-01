# agent (llvm-cov) — 2026-11 coverage ledger

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentage is not the acceptance number.

**Last measurement**: `test/cov-agent` at `4ea35a180` (cut from `origin/dev-huichen@79bd2bd37`),
Rust **1.99.1** (`rust-toolchain.toml`), macOS arm64.

**Result**: `DA:0` = **1408** lines in **131** files under `agent/src` (82 955 executable,
98.30 % per-line). This wave cleared **265** of the 1673 it started from. Of the 1416:

| state | lines |
|---|---:|
| **waived** (mechanism + falsifier named below) | **89** |
| `open` — **no test and no justification yet** | **1327** |

Reconciliation with the generated ledger (which must sum to 1416): waived 24
(`glob_scan.rs` ignored acceptance, class `lane`) + 24 (`execution.rs` Linux helper block,
class `open` there because the line text carries no attribute) + 1 (`transport.rs:212`, class
`open`) + 2 (Windows candidates, class `W1`) + 6 (`settings.rs` serialization fallbacks, class
`open`) + 23 (`settings.rs` probe-error / host-capability arms, class `open`) = **80**; the
remaining **1336** are `open`; the duplicate-run scan row adds 9, so today it is **89** waived / **1327** open.
The ledger's own class totals are `open` 966, `a` 214, `W6` 173, `W5` 26, `W7` 8, `W1` 5,
`lane` 24 — those hints *subdivide* the 1327 open lines, they do not reduce them.

The class column of the generated ledger is a first-cut reading of each line's own text
(`?` tails, lone `}`, `tracing::`, `assert`, platform attributes). **It is not a waiver.**
A row becomes a waiver only when this file names the code that makes it unreachable *and*
how to falsify that claim — the ledger convention of `docs/testing/waiver-ledger.md`.

## Reproducing

```bash
# 1. measure (env sanitisation is mandatory; see .future/memory/coverage-llvm-cov.md)
ulimit -n 8192
unset CARGO_BUILD_TARGET CARGO_TARGET_DIR CARGO_INCREMENTAL CARGO_PROFILE_DEV_DEBUG RUSTFLAGS
export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"
export HOME="$PWD/target/test-home"
cargo llvm-cov -p future-agent --no-report
cargo llvm-cov report -p future-agent --lcov --output-path agent-lcov.info

# 2. per-line ledger + its own check (both work from the lcov alone)
python3 coverage/2026-11/after/agent/ledger-gen.py --lcov agent-lcov.info --root . --out ledger.md
python3 coverage/2026-11/after/agent/ledger-gen.py --check --lcov agent-lcov.info --out ledger.md
# → "ledger=clean" means every DA:0 line is named here and no line is stale
```

The working copies (`ledger-gen.py`, `ledger.md`, the lcov files) live under the gitignored
`coverage/2026-11/after/agent/`, which is why this file exists in `docs/`: the numbers above
are reproducible from the commands shown, with nothing but the lcov file.

## Waived (89 lines)

Category names follow `.future/memory/coverage-llvm-cov.md` (W1 cfg(windows) · W2 defensive
dead arm · W3 OS failure injection · W4 race window · W5 attribution artifact · W6 ghost line
· W7 mock closure · W8 dead producer). **No row here needs a network or a model**, and none is W4: the only
environment-class rows are the two W3 settings rows, each of which names the host capability
it needs.

| # | file:line | lines | category | construct | falsifier |
|---|---|---:|---|---|---|
| 1 | `tools/shell/execution.rs:716-745` | 24 | **lane (Linux runtime)** | the `if expects_report { … }` block that parses the bubblewrap helper report | run this suite on Linux — CI's `Rust tests (Linux)` job does, and these lines are covered there |
| 2 | `sandbox/linux/glob_scan.rs:613-639` | 24 | **lane (ignored acceptance)** | body of `large_workspace_exceeds_old_node_limit_without_failing` | remove `#[ignore = "creates over 100000 entries …"]` and run it: it needs ~100 k files and minutes, which is why it is a lane rather than a test |
| 3 | `session/content/transport.rs:212` | 1 | **W2** | `Encoding::ToolArgumentsStringV1 => bail!("entry-only encoding cannot be used by a stream content request")` | none possible: see below |
| 4 | `sandbox/mod.rs:1041,1059` | 2 | **W1** (candidate) | close brace / `SandboxBackendReceipt::Unavailable` under a Windows-only path | run the suite on Windows; **not yet verified line by line** — see "what is not claimed" |
| 5 | `rpc/commands/settings.rs:14-17,19,39` | 6 | **W2** | the `serde_json::to_value(...).unwrap_or_else(&vert;» json!({"available": false, "code": "serialization_failed"}))` fallbacks in the two probe handlers | none reachable: `SandboxProbeResult` is `bool`, two `String`s and three `Option`s of `PathBuf`/`String`/`LinuxSandboxCapabilities`, whose fields are all `bool` or `String` (`sandbox/mod.rs:1067-1077`, `sandbox/linux/probe.rs:148-160`); `to_value` fails only on non-string map keys and non-finite floats, which that shape cannot produce |
| 6 | `rpc/commands/settings.rs:21-24,43-46,53-56,548-552,555-562,570-571` | 23 | **W3** (host capability) | the probe **error** arms, the Windows-only reset arm, and the "sandbox tier unavailable → Manual" fallback | the macOS probe body always returns `Ok` (`sandbox/mod.rs:1082-1100`: `available` is `/usr/bin/sandbox-exec`'s existence), so the error arms need a host whose probe fails and the fallback needs a host without that binary; falsify by running these settings tests on either |
| 7 | `rpc/session_prompt.rs:306-314` | 9 | **W2** (candidate) | the non-segmented transcript scan for a duplicate `run_id` | only reachable when a session is **not** ephemeral yet its broadcaster is **not** segmented, which the constructor cannot produce unless `configure_journal` failed (`rpc/session.rs:288` swallows that error); the same store then answers `ensure_execution_allowed` first (`session_prompt.rs:260-266`) with `PersistenceUnavailable`, so the scan is never reached. Falsified by a fixture that makes `configure_journal` fail while the store still accepts execution |

### 1. `execution.rs:716-745` — the bubblewrap helper-report block

The block is guarded by `expects_report` (`execution.rs:488-489`):

```rust
let expects_report = prepared.boundary.backend == crate::sandbox::backend::ShellBackend::LinuxBubblewrap;
```

On this host `prepare_shell_for_cwd` never returns the bubblewrap backend, so the guard is
false on every input and the block cannot run. **Pinned (negatively) by** the macOS half of
`agent/src/tools/shell` — the same commands run through the seatbelt backend and take the
other branch. Falsified by measuring on Linux; the Linux CI job covers these lines, which is
why this is a *lane* entry and not a waiver of behaviour.

### 2. `glob_scan.rs:613-639` — the 100 k-entry acceptance

`#[ignore = "creates over 100000 entries for large-workspace acceptance"]` at `glob_scan.rs:611`.
The body writes 100 010 files and walks them twice; the campaign's rule is that a test needing
a real large fixture is a lane, not a unit test. The behaviour it guards (the old 99 k node
limit) is also covered at small sizes by `grouped_patterns_share_walk_and_include_ignored_and_hidden_files`.

### 3. `transport.rs:212` — the entry-only encoding guard

A stream content range can never carry `tool_args_string_v1`:

- a published `stream_contexts` row is **immutable** — an update is refused with
  `stream contexts are immutable` (trigger);
- the multi-field authority constrains the vocabulary:
  `stream_change_content … CHECK(encoding IN ('text','json','shell_summary'))`;
- no writer pushes that encoding for a stream body — only `Text`, `Json` and `ShellSummary`
  appear at `journal.rs:201/203/322/333/365/367`.

**Pinned by** `an_entry_only_encoding_is_unrepresentable_for_a_stream_range`
(`session/content/transport_tests.rs`), which asserts both refusals. Falsified by finding any
writer or schema path that can store that encoding for a stream range — the test would fail.

## Open (1337 lines) — no waiver claimed

These need tests, not classification. Ranked by size, with the mechanism already identified:

| file | `DA:0` | mechanism |
|---|---:|---|
| `rpc/session_prompt.rs` | 52 | injected persistence failures on the admission path (`PersistenceUnavailable` arms around 1111-1120 and 1394-1402), and the deferred-input restore arms |
| `rpc/commands/settings.rs` | 12 | what is left after the 29 waived lines: `332-338` (thread-spawn failure, W3) and the per-arm refusals |
| `rpc/session.rs` | 33 | the cancelled-receipt compensation arms (505-506) and the cancellation edge |
| `tools/shell/execution.rs` | 41 | 24 of these are the waived Linux helper block; the other 17 are the exit-status edge and the timeout drain |
| `session/content/reader_recall.rs` | 39 | `find()`'s cross-page carry tail and `slice()`'s remaining UTF-8 arms |
| `session/content/recovery.rs` | 38 | recovery of partially committed message drafts |
| `agent/run_loop.rs` | 36 | tool-approval denial arms and the truncation reporter |
| `rpc/commands/session_lifecycle.rs` | 36 | clone/fork failure arms |
| `session/content/reader.rs` | 35 | the remaining header/column arms around the targeted read |
| `compaction/semantic/evidence.rs` | 33 | evidence summariser arms that need a real compaction transcript |
| `session/content/entries.rs` | 33 | staged-reset placeholders and the extension staging loop |
| `llm/schema.rs` | 32 | model-registry validation arms |
| rest (≈ 119 files) | 949 | mostly single `?` error edges; the generated ledger names each line |

### Open defect behind two of those lines

`session_prompt.rs:1111` (`save_broadcaster.fail_persistence`) and `1394-1402`
(`abandon_run_for_persistence`) are unreachable **because of a defect**, not by construction: a
transcript write refused mid-run leaves the run unsettled (no terminal within 30 s) and records
no journal failure. Reproduction is in `coverage/2026-11/after/agent/report.md` § *Open defect*.
They stay `open` here on purpose — a waiver would hide it, and the fix should be verified
against that reproduction.

Honest summary: the lane is at **98.29 %** per line, and the remaining 1337 lines are one
mechanism away (a store-fault seam, a probe seam or a fixture) rather than one line away.

## Measurement-lane notes: the four `cli_smoke` timeouts

A whole-workspace instrumented run reported four agent tests as failing or hanging. This
lane sees the same four, all in one target — `agent/tests/cli_smoke.rs` — and they are a
**measurement-lane** problem, not a product or assertion problem. No test semantics were
changed to accommodate instrumentation.

The four spawn the **real `future-agent` binary** from the test process and then wait on wall
clock:

| test | bound | what it waits for |
|---|---|---|
| `agent_is_singleton_per_user_even_on_different_ports` | **5 s** (`cli_smoke.rs:47`) then blocking reaps (`:97`) | a second instance must fail while the first holds the instance lock |
| `future_home_env_runs_a_second_isolated_instance` | **10 s** (`:126`) then blocking reaps (`:186`, `:188`) | two agents on separate `FUTURE_HOME`s |
| `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint` | **none** — `Command::output()` | a profile-timer shutdown that binds and releases the local endpoint |
| `agent_shuts_down_cleanly_on_sigint` | **30 s** for server-up (`:545`), then unbounded `child.wait()` (`:555`) | SIGINT must produce a clean exit 0 |

Two failure modes follow from that shape: the **bounded** two assert and fail
(`first agent did not acquire its instance lock`), the **unbounded** two block — one observed
hang lasted `>12 min` until the agent child was killed.

**It is contention, not instrumentation semantics.** Evidence, all on this lane:

- the same instrumented binary, target run alone: **25 passed, 0 failed in 6.94 s**;
- the same target under a concurrent 32-thread instrumented lib suite: **25 passed in 7.32 s**;
- instrumented-process startup is not slower: `future-agent --version` is **9 ms** from
  `llvm-cov-target` vs **11 ms** from `target/debug`;
- in **11 full instrumented runs** of this lane the target failed in 3 (always
  `agent_is_singleton_per_user_even_on_different_ports`, always the 5 s deadline) and hung
  once (`agent_shuts_down_cleanly_on_sigint`), in both cases while several **other worktrees'
  instrumented suites** were running on the same machine.

**Reproduce / avoid**

```bash
# the target alone — this is the shape that passes and is what a lane should measure
cargo llvm-cov -p future-agent --test cli_smoke --no-report        # 25 passed, ~7 s

# if it is already failing, re-run that target alone before believing it:
HOME=$PWD/target/test-home cargo test -p future-agent --test cli_smoke
```

- A whole-workspace instrumented run should run **one suite per machine**; two concurrent
  `cargo llvm-cov` runs (different worktrees) are what pushed this target over its 5 s bound.
- Never "fix" this by widening the deadline or removing the unbounded wait: the deadlines are
  what makes the test fail instead of hanging, and the unbounded waits are what make a hang
  visible in CI. It is a scheduling property of the measurement host.

## What is not claimed

- **No W5/W6/W7 rows are waivers.** A `?` error edge, a lone `}` or a `tracing::` call is
  *reducible* by the right test or a rustfmt-stable rewrite; that is work, not an exemption.
- **W1 is a candidate, not a verdict.** The generated class only sees a `#[cfg(windows)]`
  attribute within 40 lines, which can bleed across items (`glob_scan.rs:298` is a false
  positive of that heuristic). Each Windows row must be re-read before it is counted.
- **`cfg(unix)`/`target_os = "linux"` lines are *reachable* on this host** and are therefore
  `open`, not platform waivers — the generator was changed to say so after it first classified
  them as W1.
