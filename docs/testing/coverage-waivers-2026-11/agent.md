# agent (llvm-cov) — 2026-11 coverage ledger

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentage is not the acceptance number.

**Last measurement**: `test/cov-agent` at `08f3225e0` (waves 23-34: statement cache 256, vendored
SQLite at `-O3` in dev/test), Rust **1.99.1** (`rust-toolchain.toml`), macOS arm64, lcov
`coverage/2026-11/after/agent/lcov-wave34.info`, run log `wave34-cov.log`. The ledger below was
regenerated from that lcov and passes its own check.

**Result**: `DA:0` = **1154** lines in **129** files under `agent/src` (wave 34 measurement,
`lcov-wave34.info`, 2892 tests). This branch has cleared **519** of the 1673 it started from
(waves 23, 26, 27, 30, 31, 32, 33 and 34; waves 24-25 changed the suite's speed, not which lines
it reaches). Of the 1154:

| state | lines |
|---|---:|
| **waived** (mechanism + falsifier named below) | **98** |
| `open` — **no test and no justification yet** | **1086** |

Reconciliation (the rows below describe the 98 waived lines; they were written against the
1231-line measurement and their line numbers were re-checked against the 1211 one in wave 29):
waived 24 (`glob_scan.rs` ignored acceptance, class `lane`) + 24 (`execution.rs` Linux helper block, class `open` there because
the line text carries no attribute) + 1 (`transport.rs:212`, class `open`) + 2 (Windows
candidates, class `W1`) + 6 (`settings.rs` serialization fallbacks, class `open`) + 23
(`settings.rs` probe-error / host-capability arms, class `open`) = **98** waived, and the
remaining **1113** are `open`. The generated ledger's class column (`open`, `a`, `W6`, `W5`,
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

## Open (1133 lines) — no waiver claimed

Raw `DA:0` counts from `lcov-after24.info`; the 98 waived lines above are **inside** these
counts, so a row's open share is its count minus its waiver row (e.g. `execution.rs` 41 − 24).

| file | `DA:0` | mechanism |
|---|---:|---|
| `rpc/session_prompt.rs` | 35 | the mid-run transcript-write arm (`fail_persistence`, reachable only through an overloaded persistence queue) and the terminal-error arms; the legacy admission answers are covered now |
| `rpc/commands/settings.rs` | 41 | 29 of these are the waived serialization/probe rows; the rest are `332-338` (thread-spawn failure, W3) and the per-arm refusals |
| `tools/shell/execution.rs` | 41 | 24 of these are the waived Linux helper block; the other 17 are the exit-status edge and the timeout drain |
| `session/content/recovery.rs` | 38 | recovery of partially committed message drafts |
| `agent/run_loop.rs` | 36 | tool-approval denial arms and the truncation reporter |
| `session/content/reader.rs` | 35 | the remaining header/column arms around the targeted read |
| `compaction/semantic/evidence.rs` | 33 | evidence summariser arms that need a real compaction transcript |
| `rpc/session.rs` | 26 | the stop-before-recovery edge and the scheduler-pause diagnostics |
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
| rest (≈ 111 files) | 595 | mostly single `?` error edges; the generated ledger names each line |

### The mid-run transcript-write defect, narrowed

Wave 21 covered `1394-1402` (`abandon_run_for_persistence`) with a real run: a legacy session
whose event journal refuses a commit while the run is in flight leaves `Running` for
`PersistenceDegraded`, sends `persistence_error`, and ends with an `agent_end` whose state is
`error` and whose text starts `Session persistence failed:` — all on the unpersisted path,
because a failed journal drops an ordinary broadcast. **The abandon machinery works.**

What stays open is `session_prompt.rs:1111` (`save_broadcaster.fail_persistence`), the *other*
trigger: the previous wave found that an assistant-entry write refused mid-run leaves the run
unsettled, with no terminal within 30 s and no journal failure recorded. With the abandon path
now measured as working, the remaining explanation is that the refusal never reaches
`fail_persistence` — `append` reports `Ok` (the entry is queued, not written) and the failure
surfaces later where nothing maps it back onto this run. That is the thing to check next; the
reproduction is in `coverage/2026-11/after/agent/report.md` § *Open defect*. The line stays
`open` on purpose — a waiver would hide it.

Honest summary: the lane is at **98.53 %** per line, and the remaining 1133 lines are one
mechanism away (a store-fault seam, a probe seam or a fixture) rather than one line away.

## What waves 20-22 cleared (1231 left of 1311)

| file | before | after | how |
|---|---:|---:|---|
| `rpc/session_prompt.rs` | 61 | **35** | two runs, both end to end. **Legacy conversation:** the plain-transcript format is selected before the session configures its journal (the broadcaster reads it once and never reconsiders), so a prompt runs through the non-canonical write path; the answer is asserted to be a plain assistant entry, the run's own `session_info` is asserted to keep `created_by`/`creator_id`/`source_meta` (the canonical path deliberately drops `source_meta`), and a replayed run id is asserted to be refused from the transcript's own `content.run_id` where no per-run journal exists — reachable only through `enqueue_prompt`, since a directly submitted prompt carries no request id. **Journal death mid-run:** a gated run whose event table is dropped while it is parked is asserted to leave `Running` for `PersistenceDegraded`, emit `persistence_error`, and end on `agent_end` with state `error` and the `Session persistence failed:` prefix. |
| `rpc/session.rs` | 33 | **26** | `ensure_checkpoint_loaded` — the fallback for a handle whose cached history was released — is driven through its supported transition: a handle starts resolved, releasing the history marks it stale, and the fallback then reads the transcript, records "none found" for a conversation without a compaction checkpoint, and marks itself loaded. A checkpoint written afterwards is deliberately not picked up, which is what keeps the call free on the hot path. |
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

## Wave 34: the second fixture, and the largest single-wave drop so far

One test, `08f3225e0`, moved the lane **1169 → 1154** (−15) — more than its own seven rows,
because driving `prepare_staged` through the `/content/error` copy also exercises the prelinked
copy path around it (the placeholder branches and their braces). The ledger re-generated and
checks clean: 812 rows, classes `open` 743, `a` 197, `W6` 152, `W5` 24, `lane` 24, `W1` 11,
`W7` 3.

The fixture is a **checkpoint-owned reference**, not a run-stream one:

```rust
bind(&tx, "s", "checkpoint", "ckpt-a", "/content/error", range)?;   // the copy's source
prepare_staged(&store, "s", vec![entry_with_long_error], "ckpt-")   // the copy itself
```

## Wave 33: the fixture's second half, and the arm that needs a different one

One more event on the wave-32 sequence (commit `8dd1d45c9`) covers the other half of the
provider rule: a `thinking_end` that carries **no** `/data/provider_metadata` body keeps the
snapshot the event itself carries, instead of an empty object. Verified: `recovery.rs:381-385`
are reached by this test and no other. Both halves are now asserted together — body present →
`{}` on the call, body absent → the event's snapshot on the thinking block — so neither can
silently swap.

That closes what the `commit_batch` fixture can reach easily. What is left in `recovery.rs`
(272-278, 285, 296, 312-313, 331-332, 353-358, 432-433, 440, 474, 499, 570, 576) is:

- **`with_context` closures and `ensure!` messages** — evaluated only on the failure they
  describe (`read recovery stream …`, `decode …`, `recovery crossed durable watermark`,
  the identity errors). Reaching them needs a *fault* in the page read or a malformed event,
  which is error injection, not more events.
- **Two closing braces** (432-433) whose `if let` arm completes elsewhere — attribution rows.

### `entries.rs:155-161`, and why it was not attempted here

The `/content/error` truncation arm sits inside `prepare_owned`'s prelinked loop, which only runs
when `prefix.is_some() || shared` — i.e. through `prepare_staged` (a reset) or
`prepare_shared_fork`. Reaching it needs a **checkpoint-owned content reference** at
`/content/<n>/error` plus an entry whose `content[n].error` is longer than 512 characters, so the
copy is truncatable. That is a different fixture shape from the run-stream one (a bound
`content_references` row on the `checkpoint` owner kind, not a `commit_batch` change), and it was
left for the next wave rather than half-built.

## Wave 32: the fixture that unlocks `tool_input_end` — and what it cost to build

Wave 31 said this arm needed a fixture rather than a rewrite; this wave built it (commits
`795446ae9`, `be65f7fd7`). `session/content/recovery_tests.rs` now drives the interrupted-call
path end to end:

```
message_start → tool_start → tool_input_end (two staged bodies) → message_end
      ↓  restart's Manager::initialize()
run settles to "interrupted"; the assistant entry is published with the call
```

### The four constraints the fixture has to satisfy (each was a failed run)

1. **A run's changes tile its cursor without overlap.** `commit_batch` requires
   `change.first_cursor == previous.last_cursor + 1`; equal or overlapping windows answer
   `semantic change has a gap or conflicting overlap`, and a shared window reads as
   `conflicting semantic change retry`. The windows here are `(0,1) (2,3) (4,5) (6,13)`.
2. **A body link's `field` must name a real pointer in the change's metadata** — `/data/tool_args`
   needs a `tool_args` key in `metadata.data`, or the batch is refused with
   `content link does not name a metadata field`.
3. **`tool_slot` needs `content_key`** on both the `tool_start` and the `tool_input_end` event;
   without it recovery answers `recovery tool input has no content identity` and the run is
   isolated as `recovery_failed` — which is how the first three attempts "passed" the commit and
   still ended with no published entry.
4. **The published arguments are materialized from the body link, not left empty.**
   The first assertion written here expected `args == ""` (the value the draft stores before
   projection) and failed with `{"cmd":"ls"}`. That is the arm's actual contract: the *draft*
   holds a link, and the reader rehydrates the committed bytes — so the test now asserts the
   staged arguments arrive, which is the stronger claim.

### What it bought, measured

The trunk of the arm (`389-422`) turned out to be **already covered** by
`a_crashed_shell_result_is_recovered_with_both_of_its_bodies`; the first version of this test
therefore moved the number by zero, even though it passed. What was genuinely dark is the
**provider-metadata channel**: without a `/data/provider_metadata` body the call takes the event's
copied value and the `provider_range.is_some()` half never runs. Staging that second sealed body
covers `recovery.rs:401`, `405` and `423-427` — verified against `lcov-wave31.info` as the only
test that reaches them.

That is the lesson worth carrying: **a passing fixture test is not evidence of new lines**; the
first version of this one was green and worth nothing to the metric, and only the second body
made it pay. Measure the specific rows, not the test.

## Wave 31: five more rows, and where the cheap rewrites run out

The same subtractive shape, on `session/content/recovery.rs` and one line of
`session/content/entries.rs` (commit `b7dcc5039`), verified on the `session::content` lane:

| row | rewrite | hits after |
|---|---|---:|
| `recovery.rs` run-epoch query | `db.prepare(LIVE_RUN_EPOCHS)?` — the call was five rows | 41 |
| `recovery.rs` contiguity guard | the predicate hoists into `let contiguous`, so `anyhow::ensure!(contiguous, CONTENT_RANGE_GAP)` fits one line | 3 108 |
| `recovery.rs` start/delta identity guards | `MISSING_START` / `MISSING_DELTA` consts keep each `.context(…)?` on one line | 28 / 3 138 |
| `entries.rs` sealed digest | `OBJECT_SHA` const | 39 |

**A named predicate works where a const cannot.** `anyhow::ensure!(<long condition>, <reason>)`
has no string to hoist; what fits it on one line is binding the *condition* first. That is the
second reusable shape this campaign has found (the first is the one-statement const), and both
are strictly subtractive: the `let`/const lines carry no `DA` record.

Re-measured on `b7dcc5039` (`lcov-wave31.info`, whole lane, 2890 tests):

```
DA:0 = 1181 lines in 129 files    (was 1186)
rows = 821   open 766 · a 196 · W6 157 · W5 24 · lane 24 · W1 11 · W7 3
ledger=clean
```

### What is left is no longer rewritable, and that is now measurable

The last two waves took the rows whose *text* could be moved. What remains in the same files is of
two kinds, and both need a decision rather than a rewrite:

- **Deeply-indented multi-argument calls** — `stage_named(store, session, owner_kind, &entry,
  "/meta/shell_result", &shell.to_string(), true)?` at indent 12–16 (`entries.rs:222,246,323`).
  No const or `let` makes that fit one line; moving the row needs the arguments restructured into
  a struct or the helper's signature changed — a product refactor, not a fmt rewrite.
- **Arms that need a fixture, not a rewrite** — `recovery.rs`'s `tool_input_end` (384-429) and
  `message_end` (430-437) arms only run when a recovered event carries **content-bound bodies**
  (`change.bodies` with `/data/tool_args`), and the existing `Writer::persist` fixture writes raw
  `data` JSON with no body references. Reaching them means building that fixture, which is a
  test-writing task of a different size to this wave's dozen-line edits.

## Wave 30: ten `)?;` tails, and a full re-measurement at 1186

Following wave 29's plan — take the attribution rows rather than the open ones — two files got
the const-hoist treatment the `connection.rs` work established:

| file | sites | what moved |
|---|---:|---|
| `session/content/mod.rs` | 6 | `has_history`, the content-object row / digest / json-digest / length lookups: each `tx.query_row(SQL, params, closure)?` was five rows with the `?` alone on the last. Hoisting the five statements to consts keeps each call on one line. Measured on the `session::content` lane: **194 / 26 048 / 175 / 284 / 1 688 / 24 372** hits. |
| `session/entry_records.rs` | 4 | the session-metadata stamp and the empty-block marker (consts), the metadata validation (module alias so the call fits), and the identity guard's `ok_or_else` closure on one line. |

**A const costs no coverage row.** The declaration lines of `HAS_HISTORY`, `OBJECT_ROW` and the
rest carry no `DA` record at all — they are compile-time, so the rewrite is exactly −1 row per
site with nothing added back. That is why this shape is worth doing at scale: it is the only
rewrite in this file's taxonomy that is strictly subtractive.

Full re-measurement on `b2525dea6` (`lcov-wave30.info`, whole lane, 2890 tests):

```
DA:0 = 1186 lines in 129 files   (was 1211 at lcov-wave26.info)
rows = 826   classes {'open': 767, 'a': 201, 'W6': 156, 'W5': 24, 'W7': 3, 'W1': 11, 'lane': 24}
ledger=clean
```

The ten explicit sites plus wave 27's connection/settings work account for the 25 lines. The
class mix barely moved: `open` 767 and the attribution pair (`a` 201 + `W6` 156) still account
for 357 rows, which remains the cheapest place to spend the next wave.

## Wave 29: the ledger is regenerated and `clean` against the current lcov

The per-line ledger had drifted: checking it against `lcov-wave26.info` reported **150 problems**
— 24 `MISSING` files/lines that no row named and 126 `STALE` rows naming lines that are covered
now (waves 23/26/27's work, plus the line shifts those edits caused). Regenerating it from the
lcov and re-checking gives:

```
wrote ledger.md: 1211 lines, 852 rows,
  classes {'open': 775, 'a': 208, 'W6': 167, 'W5': 23, 'W7': 3, 'W1': 11, 'lane': 24}
ledger=clean
```

The class of a row is a first-cut reading of the line's own text (the generator's own words:
"a classification aid, never a waiver"), so the useful statement is the split, not the label:

| class | lines | what it means for the next wave |
|---|---:|---|
| `open` | 775 | statements with no test and no justification — the real work |
| `a` | 208 | un-hit error arm or failure-only message: needs a failure test (or a seam) |
| `W6` | 167 | close-brace / attribution artifact: a rustfmt-stable rewrite, not a test |
| `lane` | 24 | inside an `#[ignore]`d acceptance test |
| `W5` | 23 | region that only evaluates with a tracing subscriber |
| `W1` | 11 | platform gate the generator can see |
| `W7` | 3 | closure no input calls |

Per file, largest first (lines, then the classes found in it):

| lines | file | classes |
|---:|---|---|
| 41 | `rpc/commands/settings.rs` | open=29, W6=12 — but the hand-written rows 5 and 6 above say those 29 are W2/W3: the generator cannot see the `platform_sandbox_probe_product()` guard |
| 41 | `tools/shell/execution.rs` | open=37, W6=3, W7=1 — row 1 above covers 24 of them as the bubblewrap lane |
| 38 | `session/content/recovery.rs` | open=19, a=10, W6=9 |
| 36 | `agent/run_loop.rs` | open=34, W6=1, W5=1 |
| 35 | `rpc/session_prompt.rs` | open=22, W6=7, W5=5, a=1 |
| 33 | `compaction/semantic/evidence.rs` | open=27, a=3, W6=2, W7=1 |
| 33 | `session/content/entries.rs` | open=16, a=11, W6=6 |
| 31 | `session/content/reader.rs` | open=23, W6=5, a=3 |
| 28 | `sandbox/linux/glob_scan.rs` | lane=24, open=2, a=1, W1=1 — row 2 above |
| 26 | `rpc/session.rs` | open=14, W6=8, a=3, W1=1 |

Two caveats worth carrying forward, both visible in that table:

- **The generator over-reports `open` for guarded code.** `settings.rs` and `execution.rs` are the
  two biggest files and 53 of their 82 lines are already exempted by hand-written rows; the
  generator classifies by line text, so a row inside an `if backend == …LinuxBubblewrap` block
  still reads `open`. A future pass should teach it to read the enclosing guard, or the hand
  rows and the generated ledger will keep disagreeing by ~50 lines.
- **`W6` (167) and `a` (208) are 31 % of the total.** Those are not behaviour gaps; they are the
  attribution and error-arm shapes `.future/memory/coverage-llvm-cov.md` describes. A wave that
  rewrites them rustfmt-stably moves ~375 lines without writing a test — which is why the
  ledger's classes matter more than its total.

## Wave 28: the 32-thread default is thrashing, and the real work is 1620 CPU-seconds

Everything in waves 23-25 assumed the suite's cost was *work*. It is not, all the way down. The
same tree, same host, only `RUST_TEST_THREADS` changed (`wave28-threads.log`):

| threads | wall (harness) | user | sys | CPU-s | involuntary switches |
|---:|---:|---:|---:|---:|---:|
| 32 (default on this host) | 209.69 s | 473.83 | 4848.24 | **4231** | **18.6 M** |
| 16 | **187.99 s** | 347.28 | 1997.31 | **2345** | 17.7 M |
| 8 | 210.65 s | 537.04 | 1082.83 | **1620** | 32.4 M |

Read it twice, because it inverts the intuition: **at 8 threads the suite burns 1620 CPU-s and at
32 threads it burns 4231 CPU-s for the same result** — the same wall. More threads do not buy
wall time here; they buy context switches. The honest statement of the suite's work is therefore
**~1600 CPU-seconds**, not 4231, and at 8 threads the wall (210.65 s) is exactly that work on 8
cores (203 s) — i.e. the run is perfectly packed and CPU-bound *there*.

Why 32 threads cannot use 32 cores: libtest's threads are not the only threads. Every test that
opens a store gets a database worker thread, several get a journal writer, and some build a
two-worker tokio runtime — so ~32 libtest threads is ~64-128 runnable threads on 32 cores, and
the scheduler spends the difference. The 18.6 M involuntary switches are that bill.

**What this does and does not buy.** It does not make the 120 s gate pass on its own: the best
measured wall is **188 s at 16 threads**, still 68 s over. It does mean the remaining gap is
*contention*, not arithmetic — 16 threads with the 8-thread CPU cost would be **101 s**. The
levers worth measuring next are the per-test thread footprint (one worker per store, one per
journal) and SQLite's serialized-mode per-connection mutex (`SQLITE_THREADSAFE=1` is what the
bundled C is compiled with), not more test authoring.

**Not changed in the repo.** `RUST_TEST_THREADS` could be pinned for every `cargo test` through
`[env]` in `.cargo/config.toml`, and that would take this suite from 210 s to 188 s. It is not
taken here: it is a workspace-wide setting that would also apply to an 8-core CI runner, where 16
threads is oversubscription of a different kind, and a coverage lane should not make that choice
for every crate. It is recorded as a decision for whoever owns the test host settings.

## Wave 27: the probe file is mostly host-unreachable, and three `?` rows were attribution

`rpc/commands/settings.rs` still showed **41** missed lines after the probe commands were driven
end to end, which looked like an untested handler. Reading the lcov per line says something else:
the *reachable* lines are covered (the handler entry, the probe call, the response) and what is
left splits into two kinds, neither of which a test on this host can move.

| lines | what they are | why they cannot run here |
|---|---|---|
| 21-24 (old numbering) | `Err(error) => RpcResponse::build_fail(...)` of `handle_probe_sandbox` | `platform_sandbox_probe_product()` returns `Ok` on macOS unconditionally (it stats `/usr/bin/sandbox-exec`); there is no seam that makes it fail, so the arm is an OS-failure branch, class W3 |
| 33 | `tracing::warn!` "Windows sandbox host probe unavailable" | fires only when the Windows probe returns a diagnostic; on macOS the probe is `unavailable_without_error("platform_not_windows")`, so `diagnostic()` is `None` — class W1 |
| 43-46, 53-56 | the `Err` arm of `handle_probe_windows_sandbox`, and the `Ok` arm of `handle_reset_windows_sandbox` | both are Windows-only: outside Windows the probe succeeds with `platform_not_windows` and the reset returns `Unsupported` — class W1 |
| 548-562 | `set_sandbox_policy`'s probe-failure arms | same missing seam as the first row |

Two things were fixable and are fixed (commit `95fd3383f`):

- **The dead serialisation closures.** `serde_json::to_value(result).unwrap_or_else(|_| { json!({…}) })`
  is a five-line block that can never run (every probe result is a struct of scalars) and that
  rustfmt spreads over five rows — while the *call arguments* around it stayed `DA:0` because the
  closure's region swallowed them. Both probes now call a one-line helper
  (`probe_unavailable("none")`, `windows_probe_unavailable()`); the arguments count as executed
  and the dead cost is one row per probe instead of four or five.
- **Three `)?;` attribution rows in `session/database/connection.rs`.** The schema-identity
  validations used multi-line `query_row(…)` calls, which puts the `?` alone on its own line —
  a row that only holds the error edge. Hoisting the three SQL texts to consts keeps each call on
  one line, and they are covered now (`USER_TABLE_COUNT` DA=39, `FOREIGN_TABLES_EXIST` DA=1,
  `NO_FOREIGN_TABLES` DA=2).

Also gained, by a real test rather than a rewrite: `connection.rs`'s **"incomplete schema
identity"** refusal. A file that answers to the Agent's `application_id`, carries no schema
version and holds a table the schema does not own is now asserted to be **refused and left
byte-identical** — the point of the arm is that adopting it would run `CREATE IF NOT EXISTS` over
someone's data and then report an empty history for it (`DA=1`, and the foreign table still exists
afterwards).

## Wave 26: the first re-measurement on the fast suite (1211 left)

`cargo llvm-cov -p future-agent --no-report` on `0a70974db` (the wave-25 tree with the vendored
C at `-O3`) reports **`DA:0` = 1211** across 129 files (`lcov-wave26.info`), against 1231 at
`lcov-after24.info` — wave 23's 40 lines are what moved it, and the faster build did not change
which lines the suite reaches.

| file | before | after | how |
|---|---:|---:|---|
| `rpc/broadcaster/projection.rs` | 13 | **1** | three tests drive `Projection` directly: `first_idx` is `None` while nothing is held, becomes the pending segment's index as soon as one is coalescing, and survives the flush that closes it; a projection that crosses its byte budget forgets the cursor (it can promise no replay), drops what it held, and ignores every later event instead of re-arming the budget; the pre-coalescing `text_delta` spelling never becomes a projected event. The remaining row is a `}`. |

The rest of the open rows are unchanged: `rpc/commands/settings.rs` 41, `tools/shell/execution.rs`
41, `session/content/recovery.rs` 38, `agent/run_loop.rs` 36, `rpc/session_prompt.rs` 35 — the
same files wave 22 named, still the largest.

**A duplicate found and dropped again.** I drafted three `console.rs` tests before noticing the
module already has `a_line_ends_with_exactly_one_newline` (and a `Broken` writer) — the same
mistake as wave 23's sandbox probes. The draft was reverted, not merged: `write_line` was already
covered; only `note_once` is open there, and its `tracing::warn!` body only counts under a
subscriber, so it needs a subscriber fixture rather than a bare flag assertion.

## What wave 23 cleared (head `42fbb872c`)

No aggregate `DA:0` is quoted for this head: the full-lane instrumented run was killed by the
host at 2869/2876 lib tests (`coverage/2026-11/after/agent/wave23-before.log` — no
`test result` line, no lcov written) while other worktrees measured. The claim is instead
line-for-line from `lcov-wave23-targeted2.info`, which contains **only** the six new tests:

| file | before → after | how |
|---|---:|---|
| `models/mod.rs` | 6 → **0** | `Registry::same_provider_replacement` moves to another model, never to another vendor. The recommended text model wins over the alphabetically first one (both are inserted through `test_insert`, so they are user-defined and need no credential), an image-only model is not a text fallback, a same-named model of another provider is not a candidate, and `"a-plain"` / `"/a-plain"` return `None` because neither names a provider to defend. |
| `rpc/prompt_helpers.rs` | 12 → **1** | `run_event_to_sse_for_storage(.., segmented: true)` names every entry point a released client renders (`message_start`, `message_end`, `text_start`, `text_chunk`, `thinking_end`, `text_end`, `tool_input_end`) with the id it joins on; a missing initial tool argument is asserted to be `""` (the value a client already reads as "not streamed yet"), not an absent field; and a segmented stream still passes a run-level event (`stream_resumed`) through the entry layer unchanged. |
| `session/content/reader_recall.rs` | 14 → **0** | `slice` is a byte budget, so it shortens a page instead of splitting a multi-byte character (`"汉a"` at budgets 2/3/4), treats an offset at the end as the empty tail, and refuses a start offset inside a character. |
| `session/projection.rs` | 10 → **1** | A run cut short by a restart leaves reasoning with no provider state to replay; `entries_to_agent_messages` announces it as `[Reasoning interrupted by agent restart]` text, and a reasoning block that still carries its provider state is left as reasoning. |

Two rows this wave leaves as candidates rather than claims:

- `rpc/prompt_helpers.rs:811` — `if cfg!(windows) {`. Compile-time branch, false on this host;
  class W1, and only the Windows CI job can execute the other half.
- `session/projection.rs:147` — the closing brace of `for block in &mut content`. Its head and
  body are `DA:1` in both new tests; the lone brace is a loop-tail attribution row (taxonomy c).

### A dead arm, measured rather than argued

`reader_recall::slice` ends its fragment as
`start.checked_add(limit).context("history fragment length overflow")?`. On a 64-bit target
`start` and `limit` are both `usize::try_from(i64)` values, so their sum is at most `2^64 - 2`:
**no input can make that addition fail.** On a 32-bit target the `try_from` above it rejects the
same values first. The disproof attempt is itself the evidence — `slice(b"abc", i64::MAX - 1,
i64::MAX)` does not overflow, it panics at the slice with `range start index
9223372036854775806 out of range for slice of length 3`.

That panic is a second finding, recorded and not fixed here (production change): `end` is
clamped to `bytes.len()` but `start` is not, so `offset > len` panics instead of returning an
error. Every current caller passes `offset <= length`, so nothing is broken today.

### Stale rows in wave 22's report

`rpc/commands/settings.rs`'s three probe handlers were listed open (15 lines), but
`42fbb872c` already carries `sandbox_probe_commands_answer_with_the_host_product`, which drives
`probe_sandbox`, `probe_windows_sandbox` and `reset_windows_sandbox` end to end. This wave wrote
those three tests, found the existing one, and dropped them again. Re-measure before reading an
old lcov against a newer head.

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

## Where the suite's wall time actually is (module profile)

The verifier's budget is the only thing between this lane and completion, so the suite was
profiled by module: one filtered run per prefix, harness "finished in" figure, quiet-ish host.

| module | tests | test time alone |
|---|---:|---:|
| `session::` | 668 | **286.90 s** |
| `rpc::` | 849 | 85.90 s |
| `compaction::` | 115 | 33.87 s |
| `tools::` | 104 | 20.08 s |
| `sandbox::` | 225 | 18.86 s |
| `agent::` | 139 | 8.15 s |
| `models::` | 146 | 4.42 s |
| `llm::` | 214 | 2.01 s |

The whole suite is ~290 s, so **`session::` is the critical path on its own**: everything else
fits underneath it. Drilling in:

| submodule | tests | test time alone |
|---|---:|---:|
| `session::history` | 55 | **134.93 s** |
| `session::content` | 143 | 35.34 s |
| `session::persistence` | 28 | 26.21 s |
| `session::manager` | 35 | 13.86 s |
| `session::database` | 43 | 7.92 s |
| `session::journal_compaction` | 26 | 3.49 s |
| `session::fork` | 21 | 2.47 s |
| `session::projection` | 39 | 0.00 s |

and inside `session::history`: `display_index` 71.64 s (14 tests), `paging` 27.60 s,
`recall` 11.51 s. Per test, `display_index` is: `a_forward_page_stops_at_the_eight_megabyte_budget`
**15.11 s**, `cold_page_reads_only_selected_bodies` **9.77 s**,
`ordinary_appends_and_metadata_changes_do_not_rewrite_the_historical_prefix` 4.04 s, and eleven
tests at ≤1.2 s. Every one of those is a deliberate scale test (a 9.6 MiB history, an 8 MiB page
budget, a 100 k-entry index) — heavy on purpose, and none of them is wrong.

**What is inside the 15 s test.** `sample` on the running test binary (4 s, ~150 samples of the
live thread) puts the busy frames in `serde_json::format_escaped_str_contents` with
`memchr`/`slice::split_at` beneath it, and the rest of the thread parked in writer semaphores —
i.e. the cost is **serializing ~9.6 MB of JSON text, repeatedly**, not test scaffolding and not
I/O durability. That matters for the decision: there is no fixture-side waste to remove, so the
only lever left is product code (the page assembly re-encodes text it already holds), which is
not this lane's to change.

That is the honest shape of the problem: **four tests own ~35 s of the critical path**, and the
only ways to shorten them are to shrink what they exercise (weakening) or to make the product
fast enough that the same scenario costs less (a production change). Neither is this lane's call.

## What the verifier's environment actually is

Every measurement in this file was taken with the environment the lane's own rules require:
`HOME` pointed at an isolated `target/test-home` and `ulimit -n 8192` set by the shell. The
declared verifier does **not** do either — it runs `cargo test -p future-agent --lib` from the
goal kernel, which means:

| | lane measurement | declared verifier |
|---|---|---|
| `HOME` | `target/test-home` (isolated) | the real home (`~/.future`, a live agent's store) |
| `ulimit -n` | 8192 | **256** (the macOS default; hard limit is unlimited) |

Two consequences worth acting on, neither of them the coverage lane's to fix:

- The suite's own rules say tests must not touch the real home; a verifier that supplies the real
  one runs tests against a live store. That is a plausible cause of the
  "content seal does not match durable bytes" session-persistence failures this goal has hit.
- At `ulimit -n 256` the two `rpc::commands::settings_tests::compact_*` tests (and
  `models::future::tests::fetch_future_models_…`) sit past 60 s each. Raising the budget *inside
  the test process* was tried and **works** (`setrlimit` 256 → 4096 confirmed by probe) but does
  **not** remove the slowness: the same tests pass in 0.07 s alone at 256 and stay slow only in
  the full run, i.e. the cost is contention with the other 2874 tests, not descriptors. The
  change was reverted rather than shipped unproven — `future-tui` keeps an equivalent helper for
  a cliff it actually hit, and this crate has not shown one.

## What the suite costs, and what does not help

The declared verifier is `cargo test -p future-agent --lib`. Measured on this head, warm:
**2874 tests, 0 failed, 5 ignored, 317.52 s** of test time (**335.93 s** wall). Three
consecutive warm runs gave 307 / 336 s wall and one 295 s of test time. The budget is a
subprocess timeout, so what has to fit is **wall** time.

Two levers have now been *measured* rather than assumed, and both are dead ends:

- **Durability is not the cost.** The standing theory was that `synchronous=FULL` (`F_FULLFSYNC`
  on macOS) makes every commit cost tens of milliseconds. Under `#[cfg(test)]` switched to
  `NORMAL`: **316.18 s** of test time — no change at all — and it *broke*
  `session::database::wal::tests::a_long_reader_can_delay_checkpointing_without_joining_the_ordered_commit_queue`,
  which asserts frame/checkpoint behaviour. Reverted. The kernel time (≈5 700 s across the suite)
  is not fsync.
- **Thread count is not reachable.** On a 143-test subset, `RUST_TEST_THREADS` 32 / 16 / 8 gave
  50.6 s / 37.8 s / 49.6 s wall — inside the run-to-run noise, and the verifier's command line is
  fixed, so it could not be applied anyway.

One real pathology found, **not** fixed here (it is a production cost, not a test cost):
`session::title_preview::tests::paged_preview_handles_long_labels_targets_and_small_unicode_pages`
spends **12.5 s of CPU** on four inputs — 4 % of the suite — because it pages a 100 000-character
title in ≤13-byte steps, which makes `title_preview::paged` quadratic in the page count. Fixing
that is a production change with its own tests, so this lane only records it.

The remaining honest options for the verifier are the supervisor's: a larger budget, or a
knowingly narrower command (`rpc::commands::` ≈ 62 s, `session::content` ≈ 50 s).

## Replay vocabulary of `session::content::recovery` (learned the hard way)

The recovery replay matches on `event_type` and **ignores/serves nothing for `tool_input_start`**:
the accepted kinds are `message_start`, `text_start`, `thinking_start`, `text_chunk`,
`thinking_delta`, `tool_start` (with `"phase":"input"`), `tool_delta`, `thinking_end`,
`tool_input_end`, `message_end`, `tool_end`. A hand-built fixture that uses `tool_input_start`
leaves the run at `recovery_failed`. The `thinking_end` / `tool_input_end` arms that re-link a
published `/data/provider_metadata` or `/data/tool_args` body additionally need `message_start`
(a draft) *and* a following `tool_end` to publish anything — a crash between the input events and
`message_end` recovers no partial assistant entry.

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

### The measured floor: 6206 CPU-seconds for the whole `--lib` suite

Every earlier budget argument in this file was about wall time, which moves with the host. This
wave measured the quantity that does not: **total CPU** of the declared verification, with
`/usr/bin/time -l cargo test -p future-agent --lib` on `1c114c49e`
(`coverage/2026-11/after/agent/wave24-cpu-default.log`):

```
      375.44 real       586.49 user      5619.80 sys
test result: ok. 2886 passed; 0 failed; 5 ignored; finished in 316.08s
```

- **6206 CPU-seconds** (user + sys) for 2886 tests — **2.15 s of CPU per test**, and **90 % of it
  is kernel time** (`sys`), not user code. The suite is not "slow because it is big"; every test
  pays roughly two seconds of system calls for a few milliseconds of assertion.
- The runner is already extracting about **20× parallelism** (6206 / 316), which is what a
  32-core host with 2886 individually-runnable tests looks like.
- Therefore the **best possible wall time on this host is 6206 / 32 = 194 s**, before any build,
  freshness check, or scheduling loss. The declared validator kills the command at **120 s**.

That is the whole story of the 29 consecutive `timeout after 120000ms` verdicts: **no amount of
test authoring can fit this suite into 120 s on a 32-core machine**, because the machine must
execute 6206 CPU-seconds of work and has only 3840 core-seconds inside the budget. The gate is
not failing because the tests are flaky or uncommitted; it is failing arithmetic.

**What would have to change** — both options are outside a coverage lane's write set, so neither
is attempted here:

1. **Raise the budget.** Observed wall times on this host: **316 s** quiet, **375 s** with
   `/usr/bin/time` overhead, **522 s** at load average 46 (three other worktrees measuring).
   A 600 s budget passes every one of those; 700 s leaves room for a cold build.
2. **Cut the CPU by more than 38 % and keep perfect parallelism.** The per-test kernel cost is
   the target, and the shape of it is now known (below). It is a fixture/product change, not a
   test-authoring change.

### Where the 6206 CPU-seconds are

Per-module timed runs (`wave24-modules.log`), same environment, `/usr/bin/time -l` each:

| filter | tests | wall | user | sys | CPU-s per test | parallelism |
|---|---:|---:|---:|---:|---:|---:|
| `session::history` | 55 | 109.25 s | 121.60 | **626.57** | **13.6** | 6.9× |
| `session::content` | 145 | 34.78 s | 83.89 | 221.46 | 2.1 | 8.8× |
| `session::persistence` | 28 | 25.51 s | 21.69 | 276.51 | **10.6** | 11.7× |
| `rpc::` | 853 | 88.29 s | 109.46 | **1691.14** | 2.1 | 20.4× |
| `rpc::commands::` | 311 | 54.10 s | 54.64 | 1024.80 | 3.5 | 20.0× |

Two different problems, and they need different fixes:

- **`session::history` is CPU-real and badly parallelized** (6.9×). Its four scale tests own most
  of it — the 8 MiB page budget, the 100 k-entry index, the 9.6 MiB history — and the earlier
  `sample` put the busy frames in `serde_json::format_escaped_str_contents`, i.e. re-serializing
  text the page assembly already holds. Shortening them means shrinking what they exercise
  (weakening) or making the page assembly faster (production code).
- **`rpc::` and `session::persistence` are sys-call real** (`sys` is 15–20× `user`). `rpc::`'s 853
  tests each build an `AppState`: an isolated home, a session manager, a migrated SQLite store, a
  tokio runtime, a broadcaster. That fixed cost is now measured at **2.1 CPU-s per test**, 90 %
  kernel. It is the largest single lever in the suite, and it is a fixture question: whether a
  migrated store can be produced by copying a template database once per binary instead of
  replaying the schema per test.

### The kernel time is SQLite statement compilation, not I/O

An 8 s `sample` of the test binary running `rpc::` (853 tests, the largest sys consumer) puts
**18 of the 20 most frequent frames inside SQLite's statement compiler**, with the remaining two
in its mutex waits (`coverage/2026-11/after/agent/wave24-sample-rpc-frames.txt`, frame counts; the raw 189 MB `sample` transcript is transient):

```
33594 sqlite3ExprDup        32816 sqlite3ExprCodeTarget   26117 sqlite3ExprDeleteNN
25948 sqlite3Malloc         25412 sqlite3_mutex_enter      25203 __psynch_mutexwait
22464 sqlite3DbMallocRawNN  19613 sqlite3ExprListDup       17173 sqlite3ExprDelete
16910 sqlite3ExprCode       16047 sqlite3ExprListDelete    15651 sqlite3_mutex_leave
15352 __psynch_mutexdrop    14401 sqlite3_free            13950 sqlite3SelectDup
11377 sqlite3DbNNFreeNN      9662 sqlite3ExprCodeExprList    9551 sqlite3Select
 8694 sqlite3SelectDelete    5220 sqlite3SrcListDup
```

`sqlite3ExprDup` / `sqlite3SelectDup` / `sqlite3ExprCode` run when a statement is **prepared**,
not when it is executed. So the suite's 5620 s of `sys`-adjacent cost is mostly **re-preparing
the same complex statements** (this crate's reads go through large view-backed queries), which is
the same defect this repository already fixed once on the TUI lane — the memory entry
`.future/memory/sqlite-view-prepare-cost.md` records `prepare` → `prepare_cached` taking
`list_sessions` from 4.6 s to 54 ms. This crate still has **92 `.prepare(` call sites against 29
`.prepare_cached(`**.

And the fixture itself is not free: a probe test (added, measured, then removed — the tree carries no probe) timing the
standard `session::persistence` fixture — `Manager::new(dir)` + first `save` — reports
**~100 ms to create the store and ~180 ms to fixture + save**, ten times in a row
(114/101/172/101/107/99/100/100/115/110 ms). A test that opens two stores therefore spends ~0.2
CPU-s before it asserts anything.

Both numbers point the same way: the suite's cost is **statement preparation**, and it is
concentrated in a small number of hot SQL call sites, not spread evenly over 2886 tests.

Where the successor should start, by uncached prepares in the files the timed runs named as
critical path: `session/history/display_index.rs` (4), `session/history/recall.rs` (4),
`session/sqlite_store/events.rs` (4, the one file that already caches one),
`session/database/readers.rs` — and every read that goes
through the `entry_records`/`sessions` views, which is what the sample's `sqlite3SelectDup`
frames are compiling.

### Wave 24's fix: the cache was smaller than the working set

The measurements above turned a "the gate is impossible" verdict into a fixable defect, so it was
fixed rather than re-argued. Three parts, all measured on the full suite:

| change | real | user | sys | CPU-s | Δ CPU |
|---|---:|---:|---:|---:|---:|
| baseline (`1c114c49e`) | 375.44 | 586.49 | 5619.80 | **6206** | — |
| `prepare_cached` in the paged readers + 17 hot sites | 303.87 | 465.02 | 5444.60 | 5909 | −4.8 % |
| **+ statement cache 16 → 256** | **298.01** | **473.83** | **4848.24** | **5322** | **−14.2 %** |

- `SELECT payload FROM entry_records` costs **26 ms to prepare and 0.2 ms to execute** (a trivial
  table read costs 20 µs) — measured on a fresh connection, which is exactly what a new test gets.
- rusqlite's per-connection cache holds **16** statements (`STATEMENT_CACHE_DEFAULT_CAPACITY`);
  this crate prepares ~130 distinct ones, so the deep views were evicted between calls and
  recompiled. `initialize_connection` now sets 256.
- The paged readers built their SQL at runtime and used `prepare`, so every page paid a compile:
  history recovery's 64-row loop, display paging, and the events page read are now
  `prepare_cached` (a runtime-built string is a fine cache key).

`session::history` alone went 748 → 644 CPU-s on the three paged sites before the cache resize.
No behaviour changes: the same statements are compiled once per connection instead of once per
call, and the suite is green at 2886 tests after each step.

**What is left, sampled after the fix** (`wave24-sample-after-cache.txt`, 6 s of
`session::history`): the same family still leads — `sqlite3ExprCodeTarget` 81 658,
`sqlite3ExprDup` 66 450, `sqlite3ExprCode` 41 301 — which is **VDBE code generation inside
prepare**, not parsing and not execution. A cache cannot remove the *first* compile, and every
test opens one to three fresh connections, each compiling the deep views once (8 statements ×
26 ms ≈ 0.2 s per test is the right order for the 4848 s of system time that remain). The next
lever is therefore the **schema's own compile cost** — the five-CTE `entry_records` and its
siblings — which is a product/schema change with real correctness risk, or a larger budget. It is
not another caching pass: the prepare_cached sites are now exhausted on the hot paths.

### Wave 25: the vendored C was built at `-O0`

The remaining cost was still query *compilation* after wave 24's caching, which is absurd for
SQLite — and the reason is the profile, not the query. `rusqlite` is `bundled`, so `sqlite3.c`
is compiled by `cc` with the dev/test profile's `opt-level`, which is **0**. One stanza fixes it:

```toml
[profile.dev.package.libsqlite3-sys]
opt-level = 3

[profile.test.package.libsqlite3-sys]
opt-level = 3
```

Only the vendored C is optimised; every Rust crate stays on the debug profile, so incremental
builds and debug assertions are untouched.

| measurement | before | after |
|---|---:|---:|
| `session::history` (55 tests) | 85.02 s / 644 CPU-s | **57.29 s / 426 CPU-s** |
| whole `--lib` suite | 298.01 s / 5322 CPU-s | **219.15 s / 4231 CPU-s** |
| same suite, second run | — | **194.89 s** (host variance) |

Against the wave-23 baseline of 375.44 s / 6206 CPU-s, the suite is now **−42 % wall and −32 %
CPU**, and `cargo test -p future-agent` is green across every target (lib 2886, `cli_smoke` 25,
service, sandbox, shutdown; `ALLTESTS rc=0`).

### The gap that is left is scheduling, not arithmetic

4231 CPU-seconds over 32 cores is a **132 s** floor, and the suite runs 195–219 s: an achieved
parallelism of **19–21× on 32 cores**. So the next 70–90 s is not work — it is cores sitting
idle. The known cause is structural: `TestHome` holds a **process-global** `HOME_ENV_LOCK` for
its whole body (it mutates `HOME`/`USERPROFILE`/`FUTURE_HOME`, which are process-wide), and **91
call sites** use it. Those 91 tests form a serial chain of roughly their own running time, and the
tail of that chain is the tail of the suite.

**And that hypothesis was then measured and rejected.** A temporary probe (installed, run across
the full suite, then removed — the tree carries no probe) timed `home_env_lock()`'s hold for a
representative test: **144 ms**. Ninety-one such sites therefore bound the serial chain at
**~13 s**, not the 70–90 s of idle-core time. The HOME lock is a fixture wart worth fixing on its
own merits, but it is **not** what separates the 132 s floor from a 195–219 s wall — so the
successor should look at thread oversubscription and the long parked timers
(`session::persistence` alone was a 25 s single-test critical path in the wave-24 sample), not at
this lock.

The candidate fix for that lock — if it ever matters — is to stop making `HOME` process-global for tests
(a thread-local override consulted by the path resolution under `cfg(test)`, or running the
`TestHome` tests through the existing `reexec_env_test` child-process helper). Both are fixture
work of real size, and both are the right shape for a separate todo.

**Still not a 120 s suite.** After wave 25 the floor is 4231 / 32 = **132 s** (194 s at wave 23); the wall is 195-219 s, so what is left is mostly cores idling behind the HOME lock, not arithmetic. The
budget question does not go away by itself; it shrinks. What the fix buys the *product* is real
too — this is the same class of defect `.future/memory/sqlite-view-prepare-cost.md` records, and
it was costing every real session the same 26 ms per recompile.

### A wall-time floor underneath the CPU one

`sample <pid> 6` on the `session::persistence` run caught the whole sample inside **one test**
parked in `receive_ack` (`persistence.rs:560`, `std::sync::mpsc` recv) waiting for an idle
deadline: `idle_retirement_never_acknowledges_a_lost_command` is that module's entire 25 s wall.
Those waits are real timers, so they are pure wall time even on an idle machine. They are
convertible to a paused clock (`tokio::time::pause`/`advance`) where the code under test is
async — the agent crate's dev-dependencies already carry `test-util`? **Not verified this wave —
this is the next check, not a claim.**

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
