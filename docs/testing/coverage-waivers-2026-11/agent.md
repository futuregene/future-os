# agent (llvm-cov) — 2026-11 coverage ledger

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentage is not the acceptance number.

**Last measurement**: `test/cov-agent` at `08a47ebd5` (cut from `origin/dev-huichen@79bd2bd37`),
Rust **1.99.1** (`rust-toolchain.toml`), macOS arm64, lcov
`coverage/2026-11/after/agent/lcov-after21.info`, run log `after21.log`.

**Result**: `DA:0` = **1264** lines in **130** files under `agent/src` (83 550 executable,
98.49 % per-line). This wave cleared **409** of the 1673 the branch started from. Of the
1264:

| state | lines |
|---|---:|
| **waived** (mechanism + falsifier named below) | **98** |
| `open` — **no test and no justification yet** | **1166** |

Reconciliation (the rows below sum to the measured total): waived 24 (`glob_scan.rs` ignored
acceptance, class `lane`) + 24 (`execution.rs` Linux helper block, class `open` there because
the line text carries no attribute) + 1 (`transport.rs:212`, class `open`) + 2 (Windows
candidates, class `W1`) + 6 (`settings.rs` serialization fallbacks, class `open`) + 23
(`settings.rs` probe-error / host-capability arms, class `open`) = **98** waived, and the
remaining **1166** are `open`. The generated ledger's class column (`open`, `a`, `W6`, `W5`,
`W7`, `W1`, `lane`) *subdivides* those open lines; it does not reduce them.

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

## Waived (98 lines)

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
| 8 | `session/content/reader_recall.rs:51-59` | 9 | **W2** (candidate) | `Field::page`'s `Source::Inline` arm (slice straight from the `message_blocks` column) | reachable only for a **format-2** entry whose block text stayed inline, i.e. whose reference did not resolve when it was written (`entry_records.rs:302-315` sets `text = NULL` exactly when the reference resolves). A pre-canonical conversation does keep inline text, but the canonical reader refuses it outright — measured: `Snapshot::recall` on a `bind_legacy_events` session answers `wrong transcript reader format`. Falsified by finding a writer path that stores inline text in a format-2 session while staging is skipped |

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

## Open (1166 lines) — no waiver claimed

Raw `DA:0` counts from `lcov-after21.info`; the 98 waived lines above are **inside** these
counts, so a row's open share is its count minus its waiver row (e.g. `execution.rs` 41 − 24).

| file | `DA:0` | mechanism |
|---|---:|---|
| `rpc/session_prompt.rs` | 61 | injected persistence failures on the admission path (`PersistenceUnavailable` arms around 1111-1120 and 1394-1402), and the deferred-input restore arms |
| `rpc/commands/settings.rs` | 41 | 29 of these are the waived serialization/probe rows; the rest are `332-338` (thread-spawn failure, W3) and the per-arm refusals |
| `tools/shell/execution.rs` | 41 | 24 of these are the waived Linux helper block; the other 17 are the exit-status edge and the timeout drain |
| `session/content/recovery.rs` | 38 | recovery of partially committed message drafts |
| `agent/run_loop.rs` | 36 | tool-approval denial arms and the truncation reporter |
| `session/content/reader.rs` | 35 | the remaining header/column arms around the targeted read |
| `compaction/semantic/evidence.rs` | 33 | evidence summariser arms that need a real compaction transcript |
| `rpc/session.rs` | 33 | the stop-before-recovery edge and the scheduler-pause diagnostics |
| `session/content/entries.rs` | 33 | staged-reset placeholders and the extension staging loop |
| `session/content/reader_recall.rs` | 32 | `find()`'s cross-page carry tail and the snippet arms |
| `sandbox/linux/glob_scan.rs` | 28 | 24 waived (ignored acceptance); the walk's own edges are open |
| `service/mod.rs` | 23 | `AgentService` accessors and the models-synced callback, which needs a live service |
| `session/content/mod.rs` | 22 | `seal`/`seal_json` conflicting-digest guards and the `?` tails |
| `service/shutdown.rs` | 21 | shutdown drain arms |
| `session/content/reader_summary.rs` | 21 | the batch/summary preview fallbacks |
| `compaction/durable.rs` | 20 | the idempotent-replay arm and the checkpoint arms |
| `rpc/broadcaster/mod.rs` | 16 | admission/close arms and the live-delta split tail |
| `session/content/journal.rs` | 20 | the batch commit tail and the string-argument repair |
| `rpc/commands/session_lifecycle.rs` | 17 | fork/clone failure arms and the legacy-history scan |
| rest (≈ 111 files) | 593 | mostly single `?` error edges; the generated ledger names each line |

### Open defect behind two of those lines

`session_prompt.rs:1111` (`save_broadcaster.fail_persistence`) and `1394-1402`
(`abandon_run_for_persistence`) are unreachable **because of a defect**, not by construction: a
transcript write refused mid-run leaves the run unsettled (no terminal within 30 s) and records
no journal failure. Reproduction is in `coverage/2026-11/after/agent/report.md` § *Open defect*.
They stay `open` here on purpose — a waiver would hide it, and the fix should be verified
against that reproduction.

Honest summary: the lane is at **98.49 %** per line, and the remaining 1166 lines are one
mechanism away (a store-fault seam, a probe seam or a fixture) rather than one line away.

## What wave 20 cleared (1264 left of 1311)

| file | before | after | how |
|---|---:|---:|---|
| `rpc/commands/test_support.rs` | 25 | **0** | `assert_json_eq`'s `difference` only ran when an assertion was about to fail, so all of it was dark. Eight tests drive it through a failing comparison and assert the path it names: nested field, array index, a key only one side carries, one document written two ways (an encoding difference, not a value difference), two different JSON strings compared as documents, a non-JSON string, and a root-type mismatch. |
| `rpc/commands/session_lifecycle.rs` | 36 | **17** | a store that **cannot be opened at all** (`agent.db` is a directory) now reaches the sessionless surfaces: journal stats, storage health, storage format, presence, the reclaim sweep and `new_session` each answer with their own refusal (code, message, `error_data.sessionId`) instead of panicking or reporting empty data. The reclaim sweep also refuses a mode marker it does not recognise without reclaiming anything, and `dryRun` is pinned as a plan: no run reclaimed, `hasMore` false, `quickCheck` null. |
| `rpc/broadcaster/mod.rs` | 20 | **16** | a canonical event whose payload is not JSON is refused before the stamping lock: the journal records `invalid event data`, and the subscriber gets only the healthy frame that preceded it. |

(`llm/mod.rs` moved 14 → 15 in the same run; that is a test-order effect on a shared arm, not
work, and is not claimed. The run itself reported one failure in
`session_reset_tests::a_reset_that_applies_but_cannot_be_reloaded_reports_reload_failed` —
pre-existing, in a file this wave did not touch, and green when run alone; see
`report.md` § *A pre-existing parallel flake*. Its own lines still counted.)

Writing the "key on one side only" case found a defect **in the helper**:
`difference` indexed the left object with `&a[key]` for a key it may not carry, which
panics `no entry found for key` and throws away the path the function exists to
produce. The absent side is now a JSON null. This is test-only code
(`rpc/commands/test_support.rs` is `#[cfg(test)]`), so no product behaviour moved.

## What waves 17–18 cleared (1321 left of 1372)

Two files were **wholly untested on the paths this wave measured**, so both are real tests
rather than line padding; a third change is an attribution fix whose behaviour is unchanged.

| file | before | after | how |
|---|---:|---:|---|
| `rpc/export_html.rs` | 29 | **1** | 12 tests: a picture with no source, a remote URL (link, never `<img>`), a plain path, a picture inside an answer card, a whitespace-only block, a user prompt carrying tool/reasoning blocks, arguments as a JSON string / as a non-object, the 96-character one-line hint with its ellipsis, an orphan error result, a result with no output, and a picture attachment that must not repeat as a chip |
| `llm/adapters/openai_chat.rs` | 25 | **5** | 8 tests: the non-chat-target refusal, tools + `tool_stream` + `temperature` + `max_completion_tokens` + DeepSeek reasoning in one body, a request that sends none of them, the gateway token encodings (string / float / unparseable / string credit cost), `is_stream_complete` before and after the `[DONE]` marker, a marker that arrives first, a wrong-typed state, and the error-chunk path with and without a message |
| `llm/schema.rs` | 4 | **2** | no new test: the two values inside the `tracing::warn!` are built as statements first, because a **method call** written directly in a macro's field list lands in a region that never counts while a plain identifier there is attributed to the call site. The report itself is covered by `report_unexpressible_levels`'s own test. |

### The one line left in `export_html.rs`

`export_html.rs:248` is the closing brace of the `if let ContentBlock::Text { … }` body inside
`render_other_turn` — taxonomy (c): the block's brace only counts when the *pattern arm*
completes, and the nested `if !text.trim().is_empty()` takes that completion. The tests already
drive this branch with non-empty text (`an_unknown_role_gets_its_own_labelled_card`), so this is
an **attribution** row, not a behaviour row. The rustfmt-stable fixes are a `let … else`
restructure or a `match` with an explicit empty arm; neither is claimed here because neither was
measured, and `cargo fmt` after the change would need its own measurement pass.

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

### Why the whole suite cannot fit a 120 s gate

Measured on a subset (`cargo test -p future-agent --lib session::content`): 143 tests,
**43.3 s wall, 82 s user, 213 s system** — 1.5 s of kernel time per test. Every test's SQLite
store is opened with `synchronous = FULL` and `wal_autocheckpoint = 0`
(`agent/src/session/database/connection.rs:110-116`), so each commit fsyncs. Scaled to the
2829-test suite that is ~7 k CPU-seconds, against the 3 840 a 32-core machine can spend in
120 s. A test-only durability relaxation is the lever; it is not taken here because it changes
what the crash-recovery tests exercise.

## What is not claimed

- **No W5/W6/W7 rows are waivers.** A `?` error edge, a lone `}` or a `tracing::` call is
  *reducible* by the right test or a rustfmt-stable rewrite; that is work, not an exemption.
- **W1 is a candidate, not a verdict.** The generated class only sees a `#[cfg(windows)]`
  attribute within 40 lines, which can bleed across items (`glob_scan.rs:298` is a false
  positive of that heuristic). Each Windows row must be re-read before it is counted.
- **`cfg(unix)`/`target_os = "linux"` lines are *reachable* on this host** and are therefore
  `open`, not platform waivers — the generator was changed to say so after it first classified
  them as W1.
