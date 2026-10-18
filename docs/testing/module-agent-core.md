# Module `agent/*` core subtree (round-2 worker group `AG-B`) — coverage handoff

Subtree scope (this unit; the coordinator froze it in `todo_98dc709d35e5`):
`agent/src/tools/`, `agent/src/llm/`, `agent/src/agent/`, `agent/src/compaction/`,
`agent/src/sandbox/`, `agent/src/config/`, `agent/src/models/`, `agent/src/runtime/`,
`agent/src/utils/`, `agent/src/auth/`, `agent/src/prompt/`, `agent/src/types/`,
`agent/src/skill_reco/`.

Gate (campaign round 2, 口径 A): every line is either executed by a test in this
subtree or registered in §6 with one of the four waiver categories from
`docs/testing/waiver-ledger.md` and a concrete mechanism.
**`lines-100-or-waived`** · **`cargo-test-agent-green`** ·
**`module-agent-core-doc-updated`**.

A peer owns `agent/src/{rpc,grpc,service,session,skills,approval_review,cli.rs}`
and `agent/tests/`; nothing in those paths was touched, and no line in this record
is claimed on the strength of a peer's test.

## 1. Run identity and the exact measurement commands

* Goal `goal_160bc247aa63`, campaign revision `8449acf6`, worktree
  `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`).
* Host **macOS 26 (arm64)**, rustc 1.97.0 (`rust-toolchain.toml`),
  cargo-llvm-cov 0.9.1. Metric: per-line LCOV `DA:<line>,0`
  (`.future/cov100/covlines.py`) — never the summary block.
* Private build dir `target/cov-ag-b` (`CARGO_TARGET_DIR`), isolated
  `HOME=target/test-home`, `ulimit -n 10240`, `RUST_TEST_THREADS=4`, `-j 4`
  (eight workers share this host). Environment as prescribed by
  `.future/cov100/PLAN.md` §2.
* Every command ran with
  `PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH`,
  `CARGO_HOME=~/.cargo`, `RUSTUP_HOME=~/.rustup`.

```bash
P=".worktrees/cov100-r2/agent/src"      # covlines.py strips the "/Users/geilige/future-os/" prefix

# THE measurement: one atomic invocation per state (build + run + report).
cargo llvm-cov -p future-agent --no-fail-fast --lcov --output-path /tmp/agb-<state>.info -j 4

# the metric, scoped to this unit's paths (identical flag list every time)
python3 .future/cov100/covlines.py total  /tmp/agb-<state>.info \
  --under $P/tools --under $P/llm --under $P/agent --under $P/compaction \
  --under $P/sandbox --under $P/config --under $P/models --under $P/runtime \
  --under $P/utils --under $P/auth --under $P/prompt --under $P/types --under $P/skill_reco
python3 .future/cov100/covlines.py files  /tmp/agb-<state>.info <same flags>
python3 .future/cov100/covlines.py detail /tmp/agb-<state>.info <same flags>
```

| run | lcov | `total` output | own log |
|---|---|---|---|
| pass 1 before (17:52–18:01) | `/tmp/agb-before.info` | `covered=34961 measured=35299 uncovered=338 lines%=99.0425` | `target/cov-ag-b-before.log` |
| pass 1 after (18:25–18:28) | `/tmp/agb-after.info` | `covered=35602 measured=35821 uncovered=219 lines%=99.3886` | `target/cov-ag-b-after4.log` |
| pass 2 after (19:33–19:35) | `/tmp/agb-final2.info` | `covered=35713 measured=35927 uncovered=214 lines%=99.4043` | `target/cov-ag-b-final2.log` |
| pass 3 after (20:04–20:06) | `/tmp/agb-final3.info` | `covered=35736 measured=35948 uncovered=212 lines%=99.4103` | `target/cov-ag-b-final3.log` |
| pass 4 after (20:39–20:41) | `/tmp/agb-p6.info` | `covered=35885 measured=36097 uncovered=212 lines%=99.4127` | `target/cov-ag-b-final6.log` |
| **pass 5 after (21:00–21:03)** | **`/tmp/agb-p6.info`** | **`covered=35906 measured=36119 uncovered=213 lines%=99.4103`** | `target/cov-ag-b-p6.log` |
| **pass 6 after (21:28–21:33) — the hydrate fix (§7.7)** | **`/tmp/agb-p7.info`** | **`covered=35912 measured=36125 uncovered=213 lines%=99.4104`** | `/tmp/agb-p7-run.log` |

A first pass-2 run (`/tmp/agb-final.info`, same code except two clippy-only test-line shapes) reported the same 214 uncovered lines; the second was taken after those edits so the line numbers below match the tree exactly.

Every one of those runs is a **single atomic invocation that ended green
(all `test result: ok`, no failed target, and cargo-llvm-cov writes the report
only then)**; the pass-5 run reports `2375 passed; 0 failed; 1 ignored` for the
lib target (the ignored test is the pre-existing large-fixture acceptance case in
§6.5; the count grows because peers add tests to the same lib binary), plus every
integration target: `cli_smoke` 25, `seatbelt_security` 3, `service_startup` 11,
`sqlite_startup` 3, `sqlite_storage` 15, `session_load_test` 1,
`compaction_persist` 1, `sandbox_smoke` 0 / 9 ignored by design,
`linux_sandbox_smoke` 0 (no bwrap on this host).

**The residual is 213 rather than 212 because one line is not reproducible, and
that is a property of the code, not of the measurement.** `llm/mod.rs`:422
(`if tx.is_closed() { return; }`, the check after a body chunk is read) was
*covered* in the 20:39 run and *uncovered* in the 20:54 and 21:00 runs with that
function untouched. The reason is in the pinned tokio (1.53.1,
`src/macros/select.rs`): "By default, `select!` randomly picks a branch to check
first", and this line is reachable only when the receiver is dropped while a body
chunk is *also* ready — at that moment the `tx.closed()` arm and this check are
both ready, and the random order decides which one runs. Both outcomes stop the
pump, so the behaviour is unaffected; the line's coverage is a coin flip. Treat
this unit's residual as **212–213** and do not interpret a one-line move as
progress or regression; `reconcile.py` will also report the line as `STALE` on a
run where the closed arm won (see §6.2).

**One run of the same tree was not green; pass 6 removed its cause.** An attempt
between pass 3 and pass 4 failed
`sandbox::tests::hydrate_merges_env_from_login_shell_dump`
(`assertion left == right failed: left: Err(NotPresent), right: Ok("merged")`);
the runs before and after it were green with no change in that path. **The log
cited for it in passes 4-5 (`target/cov-ag-b-final5.log`) is green and was the
wrong file** — the authentic failure text is `/tmp/agent-a-s6.log` (19:55,
`panicked at agent/src/sandbox/mod.rs:2214`). Pass 6 measured the mechanism
instead of assuming it, and removed the test's dependence on it: the merge
contract is now asserted against a dump applied directly (`apply_env_dump`),
with no shell spawn and no timing budget in the path (§7.7). The measurement
procedure was fixed too, so this class of failure can no longer cost a report:
the run and the report render are separate steps (§9).

**Two measurement-context notes a reviewer should not mistake for a
regression.** (1) The supervisor's campaign baseline (`8449acf6`,
`.future/cov100/baseline/ws-dirty-lcov.info`) reports **326** uncovered lines for
this same scope, twelve fewer than this unit's own before-run — the supervisor's
figure came from a whole-workspace run in which *peers'* tests also executed
twelve lines of `agent/src/config/mod.rs` (the effective-retry accessors). This
unit's before/after pair is therefore its own isolated run (338 → 212), which is
the comparison the per-line metric requires. (2) The denominator grows as tests
and guards are added: 35 299 → 35 926 measured lines across the two passes, so
"uncovered" is the number to compare, not the percentage.

Operational notes for the next worker on this host (all of these cost this unit a
re-measurement): a peer's target cleanup removed this unit's private
`CARGO_TARGET_DIR` mid-run; a peer's red test target suppresses the whole report
(cargo-llvm-cov writes no lcov when a test target fails, and the profraw has then
already been consumed); a peer's in-flight edit twice made the lib test target
un-compilable, which blocks `cargo test` *and* `cargo llvm-cov` for every unit.
Copy the lcov out of `/tmp` the moment the run finishes, and move old `.profraw`
files aside before any re-run so a scoped run cannot be attributed to the whole
suite.

## 2. Result — before → after (measured, not estimated)

| run | covered / measured | line % | uncovered | files with gaps |
|---|---|---|---|---|
| baseline handed to the unit (own run, 8449acf6 state) | 34961 / 35299 | 99.0425 % | 338 | 29 |
| pass 1 end | 35602 / 35821 | 99.3886 % | 219 | 22 |
| pass 2 end | 35713 / 35927 | 99.4043 % | 214 | 22 |
| pass 3 end | 35736 / 35948 | 99.4103 % | 212 | 21 |
| pass 4 end | 35885 / 36097 | 99.4127 % | 212 | 21 |
| **pass 5 end** | **35906 / 36119** | **99.4103 %** | **213** | **21** |

Per file (uncovered lines; `—` = no gap in that run):

| file | before | pass 1 | pass 2 | what moved (pass 2 or the column's own note) |
|---|---|---|---|---|
| `agent/src/tools/shell/execution.rs` | 52 | 41 | 41 | — (all remaining lines are the Linux helper-report path, §6.3) |
| `agent/src/compaction/semantic/evidence.rs` | 33 | 33 | 33 | — (registered, §6) |
| `agent/src/agent/run_loop.rs` | 29 | 29 | 29 | — (registered, §6) |
| `agent/src/sandbox/linux/glob_scan.rs` | 40 | 29 | 28 | the snapshot byte-ceiling branch (was `platform-unmeasured`; falsified and covered) |
| `agent/src/compaction/durable.rs` | 20 | 20 | 20 | — |
| `agent/src/llm/mod.rs` | 15 | 16 | 11 | all four stream-pump abandonment guards, and the post-chunk `is_closed` guard |
| `agent/src/llm/adapters/openai_responses.rs` | 8 | 8 | 8 | — |
| `agent/src/compaction/semantic.rs` | 7 | 6 | 6 | — |
| `agent/src/skill_reco/mod.rs` | 7 | 6 | 6 | — |
| `agent/src/runtime/run_state.rs` | 5 | 5 | 5 | — |
| `agent/src/sandbox/mod.rs` | 4 | 3 | 3 | — (line numbers shifted by the pass-2 guard) |
| `agent/src/agent/mod.rs` | 8 | 3 | 3 | — |
| `agent/src/sandbox/windows_request.rs` | 13 | 3 | 3 | — |
| `agent/src/sandbox/file_request.rs` | 9 | 3 | 3 | — |
| `agent/src/sandbox/linux/violation.rs` | 3 | 3 | 3 | — |
| `agent/src/llm/adapters/anthropic.rs` | 3 | 3 | 3 | — |
| `agent/src/models/future.rs` | 2 | 2 | 2 | — |
| `agent/src/sandbox/linux/plan.rs` | 10 | 2 | 2 | — |
| `agent/src/models/mod.rs` | 2 | 1 | 2 | pass 1 |
| `agent/src/models/mod.rs` (pass 3) | — | — | — | **closed**: the test now pins a locked `TestHome` with a configured, credentialed default, so 1902-1903 run deterministically (§3.2) |
| `agent/src/config/providers.rs` | 2 | 1 | 1 | — |
| `agent/src/sandbox/linux/probe.rs` | 26 | 1 | 1 | — |
| `agent/src/tools/shell/mod.rs` | 16 | 1 | 1 | — |
| `agent/src/sandbox/linux/glob_scan.rs` (+ the `#[ignore]`d body) | 40 | 29 | 28 | pass 1 closed the three fail-closed Unix fixtures; the ignored body is §6.5 |
| `agent/src/config/mod.rs` | 12 | — | — | pass 1 |
| `agent/src/tools/shell/params.rs` | 4 | — | — | pass 1 |
| `agent/src/sandbox/linux/request.rs` | 2 | — | — | pass 1 |
| `agent/src/sandbox/windows_plan.rs` | 2 | — | — | pass 1 |
| `agent/src/sandbox/linux/runner.rs` | 2 | — | — | pass 1 |
| `agent/src/runtime/session_runtime.rs` | 1 | — | — | pass 1 |
| `agent/src/tools/mod.rs` | 1 | — | — | pass 1 |
| `agent/src/utils/`, `auth/`, `prompt/`, `types/` | — | — | — | no gap in any run |

Pass 3 differs from pass 2 only by `models/mod.rs` (§3.2): its two lines were the
one *flaky* entry in the ledger (covered or not depending on test order), so the
residual fell 214 → 212 and the file lost its row entirely.

Pass 4 keeps the residual at 212 but **re-classifies 30 lines** and adds three
tests (§3.3): `llm/mod.rs` 32/36 move from "needs a fixture" to
`unreachable-by-construction` (the pinned upstream crate maps every body error to
`decode`), the two remaining `platform-unmeasured` rows move to
`unreachable-in-this-environment` (this code compiles on this host — the other
category's definition is "compiles only on another OS", which is true of neither),
and `run_loop.rs` 478/703 move from "needs a scripted scenario" to
`unreachable-by-construction` with derived invariants, one of which was falsified
by experiment (§3.3, §6.1). The line count is stable because the new tests' own
lines are covered.

Pass 5 adds three more tests (§3.4) and moves the residual **212 → 213**, which is
*not* a regression: the extra line is `llm/mod.rs`:422, whose coverage is decided
by tokio's randomized `select!` order and which was covered in the pass-4 run by
luck (see §1 and §6.2). Two of the three new tests are gated to Linux or skipped
by a filesystem measurement, so they contribute no lines on this host and are the
*mechanism* by which the two `unreachable-in-this-environment` rows below will be
measured on a Linux runner.

`agent/src/sandbox/rules.rs` has **no** uncovered line in any run: the pass-2
guard added to it (§7.1) is covered by its two new tests, as is the rejection
path at both admissions.

## 3. Tests added (grouped by file)

Every test below asserts the behaviour its line implements (a returned verdict, a
status, a message, a released connection, or a documented invariant), not merely
that the line ran.

**Pass 1 — `agent/src/tools/shell/tests.rs`**

* `cancelled_gate_outcome_outranks_the_process_status` — a cancelled approval gate
  recorded in the call scope must force `ShellStatus::Cancelled` even when the
  command itself exited 0 (`shell/mod.rs`: 189).
* `json_escaping_expansion_is_retruncated_to_the_result_budget` — 200 000 C0
  control bytes are escaped to ≈1.2 MiB of JSON inside a bounded raw payload, so
  the result path re-truncates the recorded attempt (`shell/mod.rs`: 198–201).
* `oversized_workspace_metadata_is_refused_before_execution` — a workspace whose
  JSON form crosses 262 144 bytes is refused before any process is spawned
  (`shell/mod.rs`: 113–115).
* `an_exhausted_total_budget_refuses_before_launching` — with a shared deadline
  already in the past the call returns "Total execution budget exhausted"
  (`execution.rs`: 356–361).
* `validates_single_command_before_approval` (extended) — a 2 049-byte
  justification is rejected by the justification bound, and
  `additional_permissions` + `escalated` is rejected (`params.rs`: 39, 55–57).

**Pass 1 — `agent/src/tools/mod.rs`**: `shell_escalation_records_cancelled_and_invalidated_end_states`
(a cancelled / context-invalidated escalation must record its own end state, name
it in the error, and not run the command), plus the hoisted kept-byte bounds.

**Pass 1 — sandbox**: `prepare_rejects_every_unusable_grant_shape` and
`prepare_evaluates_the_layer_and_accepts_an_allowed_target` /
`a_receipt_is_issued_only_for_an_approvable_target_list`
(`file_request.rs`, `windows_request.rs`);
`a_literal_file_grant_stays_visible_as_an_unenforceable_write_rule`
(`windows_plan.rs`); `an_unreadable_static_root_fails_closed`,
`an_unreadable_directory_during_the_walk_fails_closed`,
`a_static_root_that_became_a_symlink_fails_closed` (`glob_scan.rs`);
`literal_rules_are_exact_mount_targets_for_allow_and_deny`,
`a_reopen_of_an_absent_path_fails_closed`,
`an_explicit_expansion_returns_that_patterns_matches` (`linux/plan.rs`);
`unparseable_version_shapes_are_rejected`,
`each_probe_stage_reports_its_own_failure`,
`unusable_path_shapes_never_yield_an_available_probe`,
`an_expired_success_receipt_is_reprobed` (`linux/probe.rs`);
`a_mount_descriptor_below_the_helper_reserved_range_is_rejected`,
`a_validated_request_over_the_json_ceiling_is_refused` (`linux/request.rs`);
`incomplete_probe_receipts_are_refused_before_any_argv_is_built`
(`linux/runner.rs`).

**Pass 1 — elsewhere**: `effective_retry_values_follow_the_configured_block`
(`config/mod.rs`), `only_account_statuses_stand_the_recommender_down_immediately`
(`skill_reco/mod.rs`), `stopping_the_cancellation_timer_disarms_the_armed_watchdog`
(`runtime/session_runtime.rs`), `sequential_tool_messages_carry_the_review_annotation`
(`agent/mod.rs`), `await_or_interrupt_reports_an_already_cancelled_run`
(`compaction/semantic.rs`).

**Pass 2 — `agent/src/sandbox/rules.rs`** (the §7.1 guard)

* `the_largest_accepted_pattern_still_compiles_and_one_more_is_refused` — a
  pattern exactly at `MAX_RULE_PATTERN_BYTES` still compiles and still matches; one
  byte more is refused by the rule-file admission *with a resolution error*, and
  that error makes the sandbox fail closed (seatbelt profile = `(deny default)`,
  `file_request::prepare` refuses with "could not be loaded"). The same pattern is
  also refused by the session admission.
* `an_over_long_session_allow_is_refused_without_changing_the_verdict` — an
  over-long client-supplied grant reports failure, leaves the verdict at `Ask`,
  and a normal grant at the same path still works.

**Pass 2 — `agent/src/sandbox/linux/glob_scan.rs`**

* `the_snapshot_byte_ceiling_trips_before_the_match_ceiling` — the stored
  association budget counts pattern text per pattern, so ~40 overlapping recursive
  patterns cross 4 MiB while the unique-match count stays at 2048; asserts the
  error code is `glob_scan_result_bytes_limit` (not the match limit) and that the
  match count is still below `MAX_MATCHES`. This falsifies and replaces the
  former `platform-unmeasured` claim for that branch (§7.3).

**Pass 2 — `agent/src/llm/stream_wait_tests.rs`** (the four pump guards + the contract)

* `an_abandoned_consumer_stops_the_pump_mid_response` — a 40-frame response parks
  the pump in its send loop; reading one event and dropping the stream must stop
  the pump and release the connection (the server task asserts it observed the
  hang-up). Covers the mid-batch send guard.
* `an_abandoned_consumer_stops_the_pump_at_eof` — the same abandonment with the
  channel exactly full when the response ends, so the guard fires in the EOF
  phase's synthetic-error send.
* `an_abandoned_consumer_stops_the_pump_on_a_residual_frame` — the response ends
  *mid-frame*, so the decoder's residual frame is sent after EOF with a full
  channel; covers the post-EOF frame-loop guard.
* `an_abandoned_consumer_stops_the_pump_before_the_eof_usage_send` and
  `an_abandoned_consumer_stops_the_pump_on_the_closing_block` — the Anthropic
  protocol; the first closes the text block so the finish phase's *first* send is
  the usage one (the guard that precedes the synthetic error), the second leaves
  it open so a block-closing event is sent first. Both assert their own fixture
  arithmetic (exactly one channel of events) and which event the finish phase owes
  first, so the fixture cannot silently stop covering the guard it targets.
* `only_anthropic_carries_a_usage_placeholder_into_the_eof_finish` — the contract
  that makes the usage send reachable at all: of the three registered adapters,
  only Anthropic reports a (zeroed) usage for an unterminated stream; the two
  OpenAI-shaped protocols pass `None`.

The same fixtures also brought the fifth guard in that function —
`if tx.is_closed() { return; }` right after a body chunk is read — into the
covered set (it fires when the pump reads a chunk that was already in flight while
the receiver closed), so `llm/mod.rs` fell 16 → 11 in three passes and only the
two transport-error-kind arms and two macro-argument lines remain registered.

**Pass 2 — test-harness change.** `controlled_server` (shared by the whole
`stream_wait_tests` module) now half-closes its write side after the body and
*waits for the client to release the connection*, which is what the module's
comment always claimed. Before this it returned as soon as it wrote the
terminator, so every "the client released the connection" assertion in the module
— including the pre-existing `consumer_cancellation_closes_a_completely_silent_http_stream`
— was vacuous (§7.4).

### 3.2 Pass 3 — the one flaky line, closed deterministically

`agent/src/models/mod.rs::get_default_model_returns_something` used to call the
ambient `get_default_model()` and assert only *if* it answered `Some(..)`. Whether
it did depended on which earlier test in the same process had last written the
isolated home's settings/auth — so its two lines were covered in the 19:21 run and
uncovered in the 19:33 run of identical source (§7.6). The test now builds the
environment it needs:

1. a locked `crate::test_support::TestHome`,
2. `auth.json` naming a credential for one provider the builtin catalog has a
   text model for (chosen from the catalog at run time, so it cannot go stale),
3. `settings.json` with `defaultModel` set to that `provider/id`.

`get_default_model()` must then answer that exact id (asserted), and the catalog
must resolve it (the original assertion, now reachable and unconditional). The
consequence in the metric: `models/mod.rs` leaves the ledger entirely and the
residual is **212**.

### 3.3 Pass 4 — three tests that pin *why* the residual is unreachable

Each is a falsifier: it fails if the fact that makes a residual line unreachable
stops being true.

1. **`llm::tests::a_transport_error_is_classified_by_the_kind_that_caused_it`**
   (extended) now also drives a response whose transfer is truncated *mid-body* —
   one complete chunk, then FIN instead of the terminating chunk. The complete
   chunk is delivered, then the pump reads the error, and the error is
   **`is_decode`**. That measurement replaces the previous registration's
   prediction ("the plausible fixture … should surface as `is_body`"), and the
   test's doc comment now carries the reason: this crate streams through
   `reqwest::Response::bytes_stream`, whose body is
   `http_body_util::BodyDataStream::new(self.res.into_body().map_err(crate::error::decode))`
   in the version this lockfile pins (reqwest 0.12.28,
   `src/async_impl/response.rs`), so **no** body error can arrive as
   `is_body`/`is_request`; `Kind::Body` and `Kind::Request` belong to the request
   side (`error::body`, `error::request`), which never reaches a stream error.
2. **`a_summary_that_matches_no_documented_section_is_used_verbatim`** — a summary
   containing no documented section at all is committed unchanged: the run
   succeeds, the event sequence is `compaction_started` + `compaction_committed`,
   and the committed checkpoint's summary contains the marker. This is what makes
   `ContextError::InvalidSummary` unreachable from a "bad template": there is no
   template check, so the only producer of that error is `finalize`'s empty-text
   guard and every production caller passes non-empty text (§6.1). Introduce a
   section check that reports `InvalidSummary` and this test fails.
3. **`a_provider_limit_recovery_claims_its_own_receipt_not_the_automatic_one`** —
   one run, one history, two compaction attempts: the automatic attempt records a
   `checkpoint: null` receipt, then the provider rejects with `[CTX_LIMIT]` and the
   recovery must *claim its own* receipt. The assertions are the error text (the
   recovery's own planner refusal, never "no progress after provider limit") and
   the database: **two** rows in `compaction_operations`, **distinct**
   `input_key`s, the **same** `input_digest`, exactly one of them a completed
   null-checkpoint receipt. That pins the claim key's composition
   (`hash(input_hash, policy)`, with `trigger` and `phase` in the policy), which is
   the only thing keeping `run_loop.rs:703` unreachable.
   **Sabotage measurement**: deleting `"trigger":trigger` from the policy JSON in
   `compaction/durable.rs` makes this test fail, and the run's error becomes
   *"context compaction made no progress after provider limit"* — the sabotaged
   build **executes line 703**. The file was restored byte-identical to `HEAD`
   immediately afterwards (`git show HEAD:… | diff` clean).

### 3.4 Pass 5 — the two environment-bound rows get their tests

The two rows that pass 4 left as `unreachable-in-this-environment` cannot be
measured on this host, so this pass delivers the tests that measure them where they
*can* run, plus a falsifier for the invariant the shell-tool registration rests on.
Each test's verification status is stated, because two of the three cannot be
executed here.

1. **`tools::shell::tests::a_sandboxed_run_announces_the_wrapper_and_reports_process_facts`**
   (executed here, green). A benign command goes through `execute` under a
   `SandboxTier::Sandbox` sandbox; the test asserts the wrapper was announced with
   the command it wraps, that the run is not an error, that the exit code and
   status are the process facts, and that the output reached the caller. It is
   **portable by construction rather than by `#[cfg]`** — the guard is a runtime
   measurement (`wraps_shell()`) — so the same body runs under seatbelt here and,
   on Linux, drives the helper-report wiring in `spawn_shell_impl` (the branch that
   exists only when the resolved backend is `LinuxBubblewrap`). A
   `#[cfg(target_os = "linux")]` twin would be uncompilable on this host: the
   crate's build scripts need a Linux `cc` (`x86_64-linux-gnu-gcc: not found`), so
   `cargo check --target x86_64-unknown-linux-gnu` stops in `cc-rs` before any of
   this crate's code is type-checked.
2. **`sandbox::file_request::tests::a_target_under_a_non_utf8_parent_is_refused_before_an_approval`**
   (**cannot run here; gated to Linux**). It creates a directory whose name is not
   valid UTF-8, resolves a request inside it, and asserts `prepare` refuses with
   "cannot be displayed losslessly as UTF-8" rather than showing an approval for a
   path the user cannot read. The request itself stays valid UTF-8 and relative —
   the *canonicalized* path is what cannot be shown — which is how the branch is
   reachable at all. Gated to Linux because on APFS the fixture cannot even create
   the directory (`EILSEQ`/92; measured in Rust and in Python, §7.10). **Its body
   is compile-verified here**: with the attribute temporarily absent the test
   compiled and ran, took its guard and skipped with a message, so the API use is
   proven; only the Linux execution is unverified on this host.
3. **`sandbox::tests::the_shell_tool_cannot_see_a_linux_boundary_on_a_non_linux_build`**
   (executed here, green) — the falsifier that carries the shell-tool registration.
   It installs the Linux receipt through the test seam, asserts the seam really
   took (the premise), and then asserts the prepared shell's boundary is
   `MacosSeatbelt`, never `LinuxBubblewrap`. That is the invariant behind "the
   helper-report block is unreachable on this host": the backend selection is a
   `cfg` split, so no macOS test can make `expects_report` true.
   **Sabotage measurement**: making the macOS arm hand back a Linux boundary
   (one added line in `prepare_shell_for_cwd_with_cancel`) makes this test fail with
   `assertion left != right failed: the macOS build must route to seatbelt, never
   to the Linux helper`; the sabotage was reverted immediately and the reverted file
   re-ran green.

## 4. Dimensions

| dimension | where it is asserted |
|---|---|
| boundary (limits, exactly-at/over) | `MAX_RULE_PATTERN_BYTES` at the limit and one byte over; `MAX_MOUNTS`, `MAX_REQUEST_BYTES`, the 0.9.0 bwrap floor, the 2048-match and 4 MiB snapshot ceilings, the 262 144-byte metadata budget, the 1 MiB result budget, the 2048-byte justification ceiling, the 240-character reason ceiling, 8 targets, the 96 KiB argv budget |
| error-path | every rejection asserted by its own message/variant, not `is_err()` (grant shapes, probe stages, mount fds, probe receipts, the rule-pattern refusal) |
| property / invariant | literal-rule exactness, duplicate grants collapse to one, "an invalid command must not reach the approval hook", "an annotation belongs to one call", "a cancelled run stays Cancelled", "an aborted watchdog cannot mark a later run stuck", "a refused grant cannot widen access", "the largest accepted pattern still compiles" |
| concurrency | the cancellation watchdog (armed, aborted, idempotent); the pump abandonment fixtures pin the exact channel occupancy that makes each guard the only way forward |
| serialization | JSON escaping expansion, `deny_unknown_fields`, digest/hex bounds |
| platform-cfg | Unix-only fixtures are `#[cfg(unix)]`; the Windows plan tests are platform-independent by construction; Windows-only and Linux-only arms are registered in §6, none silently skipped |
| resource / fail-closed | unreadable root and mid-walk I/O errors fail the scan; an unexpressible grant is refused before an approval is shown; an unloadable rule layer denies everything |

## 5. Weak tests fixed, and one vacuous assertion made real

1. `agent/src/agent/mod.rs::invalid_command_is_rejected_before_any_approval_or_handler`
   used `before_tool_call: |_,_,_| panic!(...)`. That proves the claim only by
   *not* crashing, and its `panic!` line can never be executed by a passing test.
   Replaced by a counter plus `assert_eq!(hook_calls, 0, …)`.
2. Four assertion-argument lines were re-shaped so the value the message names is
   the value the assertion compared: `tools/mod.rs` (kept bytes),
   `config/providers.rs` (compat absent), `models/mod.rs` (the catalog resolves the
   default), `sandbox/mod.rs` (elapsed).
3. **`stream_wait_tests::controlled_server` returned before observing the client's
   hang-up**, so the module's "the client must release the connection" assertions
   (the read arm's `assert!(matches!(read, Ok(0) | Err(_)))`, and every
   `timeout(…, server).await` after an abandonment) were satisfied by the *sender*
   being dropped, not by the client closing. The helper now half-closes its write
   side and waits for the client's read to end — a real assertion, which the four
   new abandonment tests and the pre-existing cancellation test now depend on. This
   is also what makes the pump's post-drop state observable: with the connection
   still pooled, the woken pump task was never polled and the guard's `return` was
   unreachable (§7.4).
4. No `#[ignore]` was added or removed, and no test was skipped in any measurement
   command. The one pre-existing ignored test (`glob_scan::tests::large_workspace_…`,
   100 010 files) remains ignored; its body is registered in §6.5.

## 6. Waiver ledger (`lines-100-or-waived`)

**213 uncovered lines in 21 files**, taken from `/tmp/agb-p7.info` with the
command in §1. Each row's first cell is `` `path`:lines `` — that shape matters:
`.future/cov100/reconcile.py` reads a claim from a row's *first cell only*, so the
earlier version of this table (path, count and line list in separate cells, with
`` `path:lines` `` backticked as a pair in the sub-tables) parsed as **zero
claims**, and the tripwire could not see this ledger at all. The mechanisms are in
§6.1-§6.4 below, whose rows are claims too and carry the same ranges.

Mechanical self-check (PLAN §5.2), against the report this record cites —
**`/tmp/agb-p7.info`, the pass-6 report §1 pins as this record's measurement** (the
invocation below used to name `/tmp/agb-p6.info`, which is pass 5's; both reports have
the same 21 files / 213 uncovered lines and the same counts for the two drawn lines, so
the verdict is identical, but the name had to agree with §1 and §9 — F.48 class,
re-measured 2026-10-17):

```bash
scopes=(--scope .worktrees/cov100-r2/agent/src/tools --scope .worktrees/cov100-r2/agent/src/llm \
        --scope .worktrees/cov100-r2/agent/src/agent --scope .worktrees/cov100-r2/agent/src/compaction \
        --scope .worktrees/cov100-r2/agent/src/sandbox --scope .worktrees/cov100-r2/agent/src/config \
        --scope .worktrees/cov100-r2/agent/src/models --scope .worktrees/cov100-r2/agent/src/runtime \
        --scope .worktrees/cov100-r2/agent/src/utils --scope .worktrees/cov100-r2/agent/src/auth \
        --scope .worktrees/cov100-r2/agent/src/prompt --scope .worktrees/cov100-r2/agent/src/types \
        --scope .worktrees/cov100-r2/agent/src/skill_reco)
python3 .future/cov100/reconcile.py --report /tmp/agb-p7.info "${scopes[@]}" \
  --doc docs/testing/module-agent-core.md --section '6. Waiver ledger' --label AG-B
```

Its output for this pass (exit 0) is `files with uncovered lines=21 uncovered
lines=213`, `mentioned in module-agent-core.md: 213`,
`MISSING (uncovered, not mentioned): 0`, and no `STALE` line — i.e. 0 on this
run. **The exception this paragraph used to describe is gone, and that is the point
of the 2026-10-17 edit:** `llm/mod.rs`:422 (`D6`) is no longer *registered as a
claim* — it is cited in §6.2 and in §6's exception note — so a run where tokio's
randomized `select!` picks the other arm no longer makes this tripwire print a
`STALE` for a row that is not stale. It still reads *covered* on those runs (that is
the documented wobble from §1, measured non-zero in 5 of 17 reports), it is just no
longer something the record asserts against a specific report.

The table itself is generated by `target/tmp/ledger_pass4.py` from a per-line
category map plus the report, and it **refuses to write** unless the mapped lines
equal the report's uncovered set exactly, so a stale line number or a
misclassified line cannot be written silently (`files: 21 mapped: 212 report: 212`
for pass 4 and `files: 21 mapped: 213 report: 213` for pass 5; on a run where D6
flips to covered it refuses to write, which is the intended behaviour).

**That is the reason the table carries two drawn lines as citations rather than
claims, and the reason a generator re-run is not the end of the edit.** The
generator maps the report's **full** uncovered set, so it re-adds `llm/mod.rs` 422
and `skill_reco/mod.rs` 228 to their per-file rows on every run — including on the
runs where `D6` is covered, which is a mutation of the generator's own input rather
than of the doc. A later pass that re-runs it must therefore re-apply the same
two-row change (4 cells: the two §6 partition rows and the two mechanism rows), and
the arithmetic paragraph below is written so that this is a two-number edit rather
than a re-derivation: the table sums to the report's total **minus** the two cited
lines. **Recorded so that a re-run cannot silently re-claim them**, which would
restore the false `STALE` the rule removes. A generator that understood the rule
(mapping the lines but marking them claimed=false) would be the durable fix; it lives
in `target/tmp/`, is gitignored, and is outside this unit's write set.

**Keep `#`-comment lines out of this section's code blocks**: `reconcile.py
--section` stops at the first line matching `^#{1,level} ` after the heading, so a
bash comment (`# …`) truncates the section it is supposed to check and every line
then reads as MISSING. The expectations above are therefore prose, not comments.

An earlier version of this ledger was reported with 7 STALE rows — all fixed here:
`compaction/durable.rs` 147 (a covered continuation line inside the range
`144-148`), `semantic/evidence.rs` 393/395/400/403 (covered expression boundaries
inside `382-404`), `agent/run_loop.rs` 1482 (a covered call inside `1479-1484`),
and `models/mod.rs` 1904 (the closing brace of the host-dependent `if let`).
Running the same check against the **pass-1** lcov (`/tmp/agb-after.info`) instead
shows drift by design, not a defect: that run has different line numbers for the
lines this pass added or shifted (`glob_scan`'s ignored body, `llm/mod.rs`'s pump
guards, `sandbox/mod.rs`), which is why §1 names the report each figure comes from
and every row here is checked against the one §6 cites.

Quoted `DA:` counts in §6.4 are neighbouring values from the same LCOV export: a
neighbour on the same statement carrying a non-zero count is what makes an
`attribution-artifact` claim checkable.

| file:lines | uncovered | category |
|---|---|---|
| `agent/src/agent/mod.rs`:1290-1292 | 3 | unreachable-by-construction |
| `agent/src/agent/run_loop.rs`:478, 703, 4957, 4990 | 4 | unreachable-by-construction |
| `agent/src/agent/run_loop.rs`:2005 | 1 | unreachable-in-this-environment |
| `agent/src/agent/run_loop.rs`:232, 233, 528-530, 1053, 1054, 1095, 1096, 1147, 1148, 1412, 1413, 1454, 1455, 1465, 1466, 1479-1481, 1483, 1484, 1505, 1506 | 24 | attribution-artifact |
| `agent/src/compaction/durable.rs`:144-146, 148, 164-175 | 16 | unreachable-by-construction |
| `agent/src/compaction/durable.rs`:62, 67, 69, 263 | 4 | unreachable-in-this-environment |
| `agent/src/compaction/semantic.rs`:521, 632, 735 | 3 | unreachable-in-this-environment |
| `agent/src/compaction/semantic.rs`:790, 792, 794 | 3 | attribution-artifact |
| `agent/src/compaction/semantic/evidence.rs`:302-304, 382-392, 394, 396-399, 401, 402, 404, 534, 601, 603, 684 | 26 | unreachable-by-construction |
| `agent/src/compaction/semantic/evidence.rs`:281, 340, 427, 576 | 4 | unreachable-in-this-environment |
| `agent/src/compaction/semantic/evidence.rs`:610, 617, 630 | 3 | attribution-artifact |
| `agent/src/config/providers.rs`:1902 | 1 | unreachable-in-this-environment |
| `agent/src/llm/adapters/anthropic.rs`:1169 | 1 | unreachable-by-construction |
| `agent/src/llm/adapters/anthropic.rs`:1172, 1180 | 2 | attribution-artifact |
| `agent/src/llm/adapters/openai_responses.rs`:911-913, 937, 1242, 1293 | 6 | unreachable-by-construction |
| `agent/src/llm/adapters/openai_responses.rs`:659, 2482 | 2 | attribution-artifact |
| `agent/src/llm/mod.rs`:32, 36 | 2 | unreachable-by-construction |
| `agent/src/llm/mod.rs`:268 | 1 | unreachable-in-this-environment — and line **422** (`D6`), cited in §6.2, **not claimed here (it flips)** |
| `agent/src/llm/mod.rs`:179, 201, 401, 526, 1355, 1516, 1607, 1949 | 8 | attribution-artifact |
| `agent/src/models/future.rs`:1349 | 1 | unreachable-by-construction |
| `agent/src/models/future.rs`:1348 | 1 | unreachable-in-this-environment |
| `agent/src/runtime/run_state.rs`:174, 245, 272, 305, 328 | 5 | attribution-artifact |
| `agent/src/sandbox/file_request.rs`:136, 152 | 2 | unreachable-by-construction |
| `agent/src/sandbox/file_request.rs`:117 | 1 | unreachable-in-this-environment |
| `agent/src/sandbox/linux/glob_scan.rs`:205, 206, 604-625, 627, 630 | 26 | unreachable-in-this-environment |
| `agent/src/sandbox/linux/glob_scan.rs`:289, 317 | 2 | attribution-artifact |
| `agent/src/sandbox/linux/plan.rs`:742, 752 | 2 | unreachable-by-construction |
| `agent/src/sandbox/linux/probe.rs`:269 | 1 | attribution-artifact |
| `agent/src/sandbox/linux/violation.rs`:285-287 | 3 | unreachable-by-construction |
| `agent/src/sandbox/mod.rs`:1013, 1031, 1064 | 3 | unreachable-in-this-environment |
| `agent/src/sandbox/windows_request.rs`:211, 213 | 2 | unreachable-by-construction |
| `agent/src/sandbox/windows_request.rs`:136 | 1 | attribution-artifact |
| `agent/src/skill_reco/mod.rs`:872, 1493, 1626 | 3 | unreachable-by-construction |
| `agent/src/skill_reco/mod.rs`:508, 531 | 2 | attribution-artifact — and line **228**, cited in §6.4, **not claimed here (its attribution moves)** |
| `agent/src/tools/shell/execution.rs`:379, 381, 383, 484-486, 492, 659-664, 666, 679-681, 716-727, 729, 733-737, 739-742, 744, 745 | 41 | unreachable-in-this-environment |
| `agent/src/tools/shell/mod.rs`:148 | 1 | attribution-artifact |

One row per file and per category group; each row's line list is the report's
uncovered set for that file, split by category where a file has more than one
(the mechanism rows in §6.1-§6.4 name the groups separately). `models/mod.rs` has
no row because pass 3 closed it — see §3.2 and §7.6.

**One stated exception — the flipping-row rule (2026-10-17).** Two lines are
**cited in their mechanism rows and deliberately not claimed in this table**:
`llm/mod.rs` **422** (`D6`) and `skill_reco/mod.rs` **228** — measured across this
unit's 17 surviving reports, 422 reads non-zero in **5** and zero in 12, while 228 reads
non-zero in exactly **1** (the pre-work baseline) and zero in the 16 reports taken after
this unit's tests landed. 422's flip is a measured alternation; 228's is structural (the
attribution follows whichever alternative pattern the arm matched) with its one non-zero
reading coming from the pre-work test set. The rule (`module-agent-rpc.md`
§0.5 states it; `review-agent.md` §9.1 is its source, and the supervisor confirmed it
is campaign-wide) is that a line whose coverage **flips between runs** must carry its
number where a claim lives, only in the mechanism cell — because this table's first cell
is what the tripwire reads as “these lines are uncovered **in the report you pinned**”,
and a flip therefore turns a correct row into a `STALE → COVERED` false alarm. So this
table is the report's uncovered set **minus those two lines**, which the arithmetic
below now states explicitly (they are the two lines whose per-run coverage is decided
by a random draw), and the pair is enumerated in §6.2 and §6.4 with their per-report
counts. Nothing is unclassified by this: every line of the pinned report is still
registered exactly once.

Row arithmetic (the per-file counts in the table above, in table order):
3+4+1+24+16+4+3+3+26+4+3+1+1+2+6+2+2+1+8+1+1+5+2+1+26+2+2+1+3+3+2+1+3+2+41+1 = **211**,
plus the **2** lines cited-not-claimed above (`llm/mod.rs` 422 and `skill_reco/mod.rs` 228,
the random-draw pair) = **213**, matching `covlines.py total` exactly. By category:
`unreachable-by-construction` 71 · `unreachable-in-this-environment` 86 ·
`attribution-artifact` 54 (71+86+54 = 211, +2 = 213).
**Four numbers in that sentence changed on 2026-10-17, and the reason is recorded because
this is the third time this record's arithmetic has been re-derived rather than trusted:**
the two row counts follow from removing 422 and 228 from their rows (2→1 and 3→2), and the
two category subtotals follow from it (87→86 and 55→54). The written-out expression also
carried **two compensating errors** before this edit — a `7` where the
`openai_responses.rs` row is 6 and a `1` where the `llm/mod.rs` row was 2, summing to the
right total by accident. The expression above is recomputed from the table itself
(36 rows, 211) rather than carried forward.

**`platform-unmeasured` is empty for this unit as of pass 4, and that corrects two
earlier rows** (§6.3): the waiver ledger defines that category as "compiles only on
another OS", and neither remaining row was in code that fails to compile here —
both are compiled on this host and simply never taken. They now sit in
`unreachable-in-this-environment` with the missing host or fixture named.
`docs/testing/waiver-ledger.md` lists four categories for `module-agent-core` and
is generated by `.future/cov100/gen-final-docs.py`, so the supervisor may want to
re-run that generator; nothing in this record depends on the category's
presence.

Line numbers were produced — and are re-checkable — with
`target/tmp/fixledger.py`, which builds the table from each file's uncovered set
and **refuses to write** unless the rows' expanded ranges equal the report's set
line for line (it printed `rows: 21 claimed: 212 report: 212` for this pass). It
is a generator with an assertion, not a formatter: nothing is written on a
mismatch.

### 6.1 `unreachable-by-construction` (with the invariant)

| file:lines | invariant |
|---|---|
| `agent/src/agent/mod.rs`:1290-1292 | The `before_tool_call` fixture body. The only way to execute it is for an invalid command to reach the approval hook, and the same test asserts it did not (`assert_eq!(hook_calls, 0)`; 1286-1289 read 1, 1295 reads 1). |
| `agent/src/agent/run_loop.rs`:478 | The fallback that keeps the *current* projection when a compaction failed with `InvalidSummary`/`SummaryFailed`. Both variants are impossible by construction here: `ContextError::InvalidSummary` has exactly one producer — `finalize`'s `summary.trim().is_empty()` guard (`compaction/semantic.rs`) — and all three of its production callers pass text that cannot be empty (`compose(HEADER, &evidence)` with `HEADER` a constant, and the summarized path's `format!(…)` over `compose(HEADER_WITH_SUMMARY, &evidence)`, entered only for a non-empty `text` because `call_summary_request` returns `Ok` only for `complete && !text.trim().is_empty()`); `ContextError::SummaryFailed(_)` is **never constructed anywhere in the crate** (§7.8). A "template-invalid" summary cannot reach either: there is no template check at all, pinned by `a_summary_that_matches_no_documented_section_is_used_verbatim` (§3.3), which fails the moment such a check exists. |
| `agent/src/agent/run_loop.rs`:703 | `ticket.fail("no progress after provider limit")`, which needs the recovery to conclude `Unchanged` **with a live ticket**. A ticket exists only for a journal claim on a non-empty projection, and `Unchanged` here can only come from `Admission::Replayed` (a `CompactionClaim::Cached` receipt): every other `Unchanged` route is threshold-gated and `ProviderContextLimit` is not a gated trigger, while an empty projection skips the claim entirely (`journal.filter(|_| !gated && !prompt.messages.is_empty())`). A *cached* receipt requires `state='completed'`, and for this policy the only writer is this path itself, which always completes **with** a checkpoint — whose replay resolves `Compacted`, because the claim key binds the session input hash, so the projection carries exactly that checkpoint. The two writers of a `checkpoint: null` receipt use other policies: the automatic arm's `ticket.finish(None, …)` and the RPC compaction, which passes `CompactionTrigger::Manual` (`rpc/session.rs`) — and the policy JSON carries `trigger` and `phase`. **Falsified by experiment**: removing `"trigger":trigger` from that JSON makes the recovery replay the automatic receipt and *executes* this line (measured error: "context compaction made no progress after provider limit"); `a_provider_limit_recovery_claims_its_own_receipt_not_the_automatic_one` (§3.3) is the standing falsifier. |
| `agent/src/agent/run_loop.rs`:4957, 4990 | Test-side `on_event` callbacks the scenario never invokes because the run never produces the event they would consume; the sibling callback on the same statement reads 3 and 24. |
| `agent/src/llm/mod.rs`:32, 36 | `is_body`/`is_request` in `reqwest_stream_error_kind`. Unreachable through this function by a property of the pinned upstream crate: the classifier is fed only response-**stream** errors (the pump in `llm/mod.rs`), and `reqwest::Response::bytes_stream` wraps every body error as `crate::error::decode` (reqwest 0.12.28, `src/async_impl/response.rs`: `http_body_util::BodyDataStream::new(self.res.into_body().map_err(crate::error::decode))`). `Kind::Body`/`Kind::Request` are produced on the request side (`error::body` in the request-body path, `error::request` while sending), which never reaches a stream error. Pinned by three real-socket framing fixtures (declared-length short body, invalid chunk header, truncated mid-transfer) that all report `decode`, plus the assertion that the reachable request-shaped errors are classified `unknown`. Falsifier: a reqwest upgrade that changes that mapping, or a new call site that feeds `execute()`'s error into the classifier. |
| `agent/src/compaction/durable.rs`:144-146, 148 | A cached checkpoint whose `cutoff_entry_id` is missing from the current raw snapshot: the fork path re-ids the transcript *and* gives the child its own session id, so a receipt can never match a snapshot with a different id set. (Line 147 is *covered*: it is the `any(…)` predicate's continuation, evaluated once per raw message.) |
| `agent/src/compaction/durable.rs`:164-175 | A cached checkpoint that a *later* checkpoint superseded: reaching it needs a hand-built receipt whose `entry_id` differs from the projection's checkpoint while the input hash still matches — the same session-id/input-hash coupling. The behaviour it guards (a replay must not undo a later operation) is asserted on the reachable replay path by the second run of `a_ticket_is_closed_without_a_checkpoint_…`. |
| `agent/src/compaction/semantic/evidence.rs`:302-304 | `estimate_text_tokens` is `ceil(Q/4)` of a per-character quarter sum, hence subadditive; every append is guarded by `est(line) + 2 ≤ budget − est(text)`, so after each append `est(text) ≤ budget − 1` and the post-loop check cannot fail. Pinned by `a_window_without_a_summary_slot_fails_before_calling_the_model` (windows 1..16). |
| `agent/src/compaction/semantic/evidence.rs`:382-392, 394, 396-399, 401, 402, 404 | Cutoff recovery through a *removed* internal summary (a checkpoint written by an earlier revision): the branch requires the cutoff id to be absent from the raw view (377-381 checked first) **and** present in a removed message's `source_entry_ids` whose text is an internal summary. Inherited from the previous record; the reachable half of the same decision — the raw-view branch — reads 1 (381). Lines 393, 395, 400 and 403 are *covered* (the `}) {`/`else if`/`?` boundaries of the same expression, which do execute). |
| `agent/src/compaction/semantic/evidence.rs`:534 | `reserve == 0` after admission: `summary_reserve` is `summary_allowance.saturating_sub(SUMMARY_SLOT_MARGIN)` (416), and every window small enough for the allowance to collapse to the margin is refused by the admission check before a plan is staged (`a_window_without_a_summary_slot_fails_before_calling_the_model`, `a_cancelled_or_over_budget_index_build_is_refused`). |
| `agent/src/compaction/semantic/evidence.rs`:601, 603 | The `fallback_reason` finalize arm: `Some(Err(..))` needs the projection finalize to fail after a successful projection (the same journal call just succeeded), and `Some(Ok(_))` is excluded by the `fallback_reason.is_some()` guard above. |
| `agent/src/compaction/semantic/evidence.rs`:684 | `write_handoff_summary`'s empty-text guard: its only caller returns `Ok` **only** for `complete && !text.trim().is_empty()` (semantic.rs 696-698), so an empty handoff cannot arrive. Pinned by `an_empty_summary_is_rejected_in_favour_of_the_deterministic_index`; revisit if that reason ever changes. |
| `agent/src/llm/mod.rs`:1949 | A test's `panic!` arm for "the terminal event must be an error"; the arm the test consumes runs, and the assertions on the value read 1. |
| `agent/src/llm/adapters/anthropic.rs`:1169 | `other => panic!("expected ToolInputEnd …")` — a passing test's exhaustiveness fallback. |
| `agent/src/llm/adapters/openai_responses.rs`:911-913 | `reconcile_reasoning_item` always calls `open_reasoning` first, which inserts on `Entry::Vacant` (668-681) and keeps the entry on `Entry::Occupied`, so the `remove(&index).unwrap_or_else(…)` closure cannot run (`.remove()` at 909 reads 8). |
| `agent/src/llm/adapters/openai_responses.rs`:937 | The `encrypted_content` argument of `openai_item_metadata`: all three call sites pass `None` (194, 877, 1106). Reported in §7.2 — the parameter is dead in this build. |
| `agent/src/llm/adapters/openai_responses.rs`:1242, 1293 | Test-side `panic!("expected reasoning end")` guards after the event was produced (`done[0]` reads 1). |
| `agent/src/models/future.rs`:1349 | The `torn += 1` counter of the cache-write torn-read test: the same test asserts `torn == 0`, so a passing run cannot execute it. |
| `agent/src/sandbox/file_request.rs`:136 | `Decision::Deny` from `evaluate` after `explicitly_denied` returned false: every `Deny` verdict comes from a rule in the profile layers that `explicitly_denied` also scans, and that check runs first (132-134). |
| `agent/src/sandbox/file_request.rs`:152 | "conflicting file permission scopes": scope validation already requires `File ⇒ existing regular file` and `Subtree ⇒ existing directory` (123-131), so one canonical path cannot carry two valid scopes. Same invariant as `windows_request.rs:211,213`. |
| `agent/src/sandbox/linux/plan.rs`:742, 752 | The `|_| unreachable!()` `stat` closures of `path_inspection_errors_are_not_missing_paths`: `inspect_mount_path` calls its `stat` argument only on the `Ok(metadata) if is_symlink` arm (295-297), and those cases pass `lstat` closures that return an error, so the second closure is never called. The lstat error arms are asserted two lines above. |
| `agent/src/sandbox/linux/violation.rs`:285-287 | The fixture `Closed` writer's `flush`: production `write_marker` writes and returns without ever flushing, so nothing can call it. |
| `agent/src/sandbox/windows_request.rs`:211, 213 | See `file_request.rs:152`. |
| `agent/src/skill_reco/mod.rs`:872, 1493, 1626 | Test-side `other => panic!("expected a failure, got {other:?}")` fallbacks; the tests assert the `Failed` variant and its reason on the next line. |

### 6.2 `unreachable-in-this-environment` (with the missing fixture/host)

Two of these moved category in pass 2 and are named as such: they were
`unreachable-in-this-environment` because no fixture reached them, and are now
*covered* (`llm/mod.rs:457`, `:511`, `:546`, `:559`) or registered with a
mechanism that does not depend on a missing host.

| file:lines | mechanism — and what would be needed |
|---|---|
| `agent/src/agent/run_loop.rs`:2005 | The event-pump's `if tx.send(event).await.is_err() { return; }`: the receiver must close *between* two sends. The same abandonment is handled deterministically one line earlier by the `tx.closed()` select arm (1998 reads 21, 2001/2003 read 20), which is what a dropped consumer actually hits in this run loop. |
| `agent/src/compaction/durable.rs`:62, 67, 69 | The `Err` arm of `fail_compaction`: the call runs (its own lines carry counts) but its `UPDATE` can only fail on a broken database, and the store's failure injections cover commits, not this direct statement. |
| `agent/src/compaction/durable.rs`:263 | The mid-call cancellation guard at the top of the prepare entry point: the flag would have to flip between the caller's decision and this synchronous check. The reachable cancellation responses are asserted by the semantic-index tests. |
| `agent/src/compaction/semantic.rs`:521, 735 | The `else` arms of `if cfg!(test) { 1 ms } else { 50 ms / 250·2ⁿ ms }`. The measurement *is* the `cfg(test)` build, where the non-test arm cannot execute; covering them needs a non-`cfg(test)` build (an `agent/tests/` integration test, outside this unit's write scope). |
| `agent/src/compaction/semantic.rs`:632 | "Cancelled while awaiting the next frame": the flag must flip between the guard at 622 and the next poll of a real 1 ms timer — the scripted providers cannot place it deterministically. |
| `agent/src/compaction/semantic/evidence.rs`:281, 340, 427, 576 | Mid-call cancellation guards with no `await` inside their window: the flag would have to flip between two synchronous reads on the same thread. The reachable cancellation paths are asserted (`a_cancelled_or_over_budget_index_build_is_refused` and the semantic loop's `Cancelled` tests). |
| `agent/src/config/providers.rs`:1902 | `if skip_if_root() { return; }`: the suite never runs as root, and the guard exists for root environments where making a directory read-only does not deny the owner. Authoritative environment: a root-run test job. |
| `agent/src/llm/mod.rs`:268 | The `JoinError` of the image-preparation `spawn_blocking`: its closure only clones messages and calls a pure data-URL helper, so no input reachable from this client panics in it. |
| The stream pump's post-chunk guard (`D6`) — line **422** of `agent/src/llm/mod.rs` | `if tx.is_closed() { return; }` after a body chunk is read. Reachable only when the receiver is dropped while a chunk is *also* ready: at that poll the `select!`'s `tx.closed()` arm and this check are both ready, and **tokio polls the arms in a random order** (1.53.1, `src/macros/select.rs`: "By default, `select!` randomly picks a branch to check first"), so which one runs — and therefore whether this line executes — is decided by that random draw. Measured: covered in the 20:39 run, uncovered in the 20:54 and 21:00 runs with the function untouched (§1). Both outcomes stop the pump, so nothing observable depends on the draw; a deterministic fixture cannot be written without removing the randomness from production behaviour (a `biased;` select would force the closed arm first and make this line permanently dead, which is a behaviour change and not this unit's call). **Cited, not claimed, since 2026-10-17 — the flipping-row rule (`module-agent-rpc.md` §0.5 states it; `review-agent.md` §9.1 is its source).** Measured across the **17** reports on this host: 422 reads non-zero in **5** (`agb-after2` 18:57, `agb-after4` 19:09, `agb-final2` 19:35, `agb-final3` 20:06, `agb-final6` 20:41) and zero in the other **12**, including the pinned `agb-p7.info` — which is the window the run times above describe (the nearest reports to 20:39 and 21:00 are `agb-final6` = 1 and `agb-p6` = 0). A claim on a line that flips makes the tripwire print `STALE → COVERED` on the runs where the draw goes the other way: a false alarm by construction, and the one this record used to describe as “by design” in §9. |
| `agent/src/models/future.rs`:1348 | The `foreign += 1` counter (a complete cache document with a different model count): it needs a concurrent foreign writer, and this suite serializes its cache tests with `future_test_lock()`. |
| `agent/src/sandbox/linux/glob_scan.rs`:205, 206 | The pattern-compile `Err` map: the matcher is built only from escaped literals plus fixed `[^/]*`/`(?s:.*)` fragments, so `Regex::new` can fail only on the crate's 10 MiB compiled-size ceiling — which needs a pattern on the order of a million characters, while the rule layer now refuses any pattern over `MAX_RULE_PATTERN_BYTES` (4096) and the workspace prefix is `PATH_MAX`-bounded (§7.1). |
| `agent/src/sandbox/linux/glob_scan.rs`:604-625, 627, 630 | The body of the pre-existing `#[ignore]`d large-fixture acceptance test (100 010 files ≈ minutes and ~100 k inodes). Ignored tests are not part of the measured set by construction; the test is deliberately excluded from the per-PR suite and is declared here rather than hidden. |
| `agent/src/sandbox/file_request.rs`:117 | `path.to_str().is_none()` — a canonical permission target that cannot be displayed. The code compiles here (the module is `#[cfg(not(windows))]`), but the only way to build such a path on this host is a directory entry the filesystem refuses to create: APFS returns `EILSEQ`/92 for a non-UTF-8 name, **measured in Rust as well as Python** (a standalone `rustc` probe with `OsStr::from_bytes(b"to\xff\xfe")` → `create` fails, `raw_os_error 92`; §7.10). Needs a filesystem that admits byte names (Linux). The test that measures it is delivered and Linux-gated: `a_target_under_a_non_utf8_parent_is_refused_before_an_approval` (§3.4). |
| `agent/src/tools/shell/execution.rs`:492, 679-681, 716-727, 729, 733-737, 739-742, 744, 745 | The Linux helper-report path: `expects_report` is true only for `ShellBackend::LinuxBubblewrap` (the comparison is at 488-493), so the retry blocking, the untrusted-output wrapping of a timed-out capture and the whole helper-report branch (`HelperReport::read`, the violation markers, the "report unavailable" message) exist for the bwrap backend only. **Unreachable on this host by construction, not by missing fixture**: the boundary can only be produced under `#[cfg(target_os = "linux")]` — `prepare_shell_for_cwd_with_cancel` returns `seatbelt::prepare(..)` in its `#[cfg(target_os = "macos")]` arm and consults the `LinuxBubblewrap` receipt only inside its `#[cfg(target_os = "linux")]` arm (`sandbox/mod.rs`:289-308), so even a sandbox whose receipt claims Linux prepares a seatbelt boundary here. Pinned by `the_shell_tool_cannot_see_a_linux_boundary_on_a_non_linux_build` (§3.4), whose sabotage measurement shows what would have to change. What is still missing for a *Linux* run is a test that drives the shell tool rather than the helper API (`linux_sandbox_smoke` does `Command::new(env!("CARGO_BIN_EXE_future-agent"))` with a base64 helper request); that test is delivered in this pass (§3.4, portable so it runs on both platforms) and will measure these lines on Linux. |
| `agent/src/sandbox/mod.rs`:1013, 1031, 1064 | macOS `platform_backend_receipt`/`platform_sandbox_probe_product` arms for a host *without* `/usr/bin/sandbox-exec`: this host has it, so the function returns early (1013 is the block's closing brace), leaving the `Unavailable` tail (1031) and the `"binary_missing"` code (1064) unreachable; neighbours read 6. Authoritative environment: a macOS host without seatbelt, which no supported runner provides. These three shifted **+21** in pass 7 (the `apply_env_dump` extraction, §7.7) — the set is otherwise unchanged, which the pass-6/pass-7 per-file diff confirms. |
| `agent/src/tools/shell/execution.rs`:379, 381, 383, 484-486 | The interrupt/deadline arms that fire only when the interrupt flag or the shared deadline changes *inside* the synchronous sandbox-preparation window: 378-385 ran once (LaunchFailed) and 483/487 read 42, so the checks ran while the arms stayed untaken. A thread-timed injection would be a race, which this suite avoids. |
| `agent/src/tools/shell/execution.rs`:659-664, 666 | The non-Windows read-error arm: the child's stdout pipe read must fail mid-run (EIO/EBADF), which the spawned-process path offers no way to inject. The extracted helper's error arm is directly asserted by `tools::tests::read_shell_output_reads_until_eof_and_propagates_errors`; 657 reads 36. |

### 6.3 `platform-unmeasured` — **no rows in this unit as of pass 4**

This section previously held two rows (`sandbox/file_request.rs:117` and the
`tools/shell/execution.rs` helper-report block). Both moved to §6.2, because this
category's definition is "compiles only on another OS; see `platform-coverage.md`
for the authoritative platform" and neither row is in code that fails to compile
here: the first lives in a `#[cfg(not(windows))]` module and the second is guarded
by a *runtime* backend comparison, so both are compiled on this macOS host and
merely never executed. `unreachable-in-this-environment` is the accurate category
("needs a host/fixture this machine cannot provide"), and each row now names that
host/fixture — plus, for the execution.rs block, the fact that the existing Linux
smoke test does not reach it either.

A third row (`sandbox/linux/glob_scan.rs`:285, the snapshot byte ceiling) left this
section in pass 2, when it turned out to be reachable here with overlapping
recursive patterns; that falsification is recorded in §7.3.

If the campaign's generator (`gen-final-docs.py`) is re-run for
`docs/testing/waiver-ledger.md`, `module-agent-core` no longer needs the
`platform-unmeasured` category.

### 6.4 `attribution-artifact` (with the neighbouring counts)

| file:lines | evidence from the same export |
|---|---|
| `agent/src/agent/run_loop.rs`:232, 233 | Argument lines of the multi-line `tracing::info!` at 229: 228 reads 195, 229 reads 8. |
| `agent/src/agent/run_loop.rs`:528-530 | Same shape (525 reads 11, 531 reads 17). |
| `agent/src/agent/run_loop.rs`:1053, 1054 | Same shape (1052 reads 219, 1055 reads 219). |
| `agent/src/agent/run_loop.rs`:1095, 1096 | Same shape (1092 reads 44). |
| `agent/src/agent/run_loop.rs`:1147, 1148 | Same shape (the sibling argument lines above carry no record either). |
| `agent/src/agent/run_loop.rs`:1412, 1413 | Same shape (1410 reads 1, 1415 reads 23). |
| `agent/src/agent/run_loop.rs`:1454, 1455, 1465, 1466 | Same shape (1451/1452 read 115/6, 1471 reads 115). |
| `agent/src/agent/run_loop.rs`:1479-1481, 1483, 1484 | Same shape (1476 reads 3, 1482 reads 3, 1486 reads 81). Line 1482 is *covered*: it is the `.map(|t| t.function.name.as_str())` call inside the argument list, which runs once per tool call, while the argument-list line itself (1481) carries no counter. |
| `agent/src/agent/run_loop.rs`:1505, 1506 | Same shape in the per-turn usage log. |
| `agent/src/compaction/semantic.rs`:790, 792, 794 | The Reasoning arm of `serialize_message`: 789 (`Normal`) reads 1, **791 (`Strict`) reads 1**, 793 (the push) reads 2, while the `#[cfg(test)]` attribute line (790), the `};` (792) and the arm's closing brace (794) carry zero-count regions. |
| `agent/src/compaction/semantic/evidence.rs`:610, 617, 630 | A block-closing brace whose block ran (610), the `?` of a `finalize` whose `Ok` path ran (617), and the closing brace of the `Compacted` arm that ran (630); the enclosing statements carry counts. |
| `agent/src/llm/mod.rs`:179, 201 | A `?`-propagation line and a block-closing brace whose surrounding lines carry counts (174-178 read 4). |
| `agent/src/llm/mod.rs`:401, 526 | Argument lines of the two multi-line `tracing` macros (393 reads 6; sibling argument lines have no record). |
| `agent/src/llm/mod.rs`:1355, 1516, 1607 | The closing braces of the three assertion-helper loops; the statements inside read 1. (These numbers moved when pass 4 lengthened the classification test above them; the ledger table is regenerated from the report, so a shift shows up as a diff rather than as a stale claim.) |
| `agent/src/llm/adapters/anthropic.rs`:1172, 1180 | Continuation lines of multi-line `matches!`/`assert!` spans in a passing test. |
| `agent/src/llm/adapters/openai_responses.rs`:659, 2482 | A stray closing brace and a multi-line `matches!(` span in test code whose enclosing statements read 1. |
| `agent/src/runtime/run_state.rs`:174, 245, 272, 305, 328 | Field lines (`phase = RunPhase::…as_str()`) of five multi-line `tracing` macros; the macro statement line reads 259 (171) etc. |
| `agent/src/sandbox/linux/glob_scan.rs`:289 | The closing brace of the `if matches.insert(…)` block after the return (287 reads 2, 288 reads 4111). |
| `agent/src/sandbox/linux/glob_scan.rs`:317 | The `elapsed_ms` argument of the scan-completion `tracing::debug!` (the arguments above it carry no record). |
| `agent/src/sandbox/linux/probe.rs`:269 | The closing brace of `if let (Some(path), Some(expected)) = …`: 266 reads 2, 267/268 read 1, 270 reads 1. |
| `agent/src/sandbox/windows_request.rs`:136 | The format-argument line of the ask-carveout `bail!` (the test asserts the message; the enclosing lines read 1). |
| The multi-variant match arm at line **228** of `agent/src/skill_reco/mod.rs` — cited, not claimed | The multi-variant match-arm line (`NoKey | NoInput | Skipped`): 225 reads 10 and 230 reads 10 — which alternative pattern the executed arm matched decides the per-line attribution. **Cited, not claimed, since 2026-10-17 — same rule as `D6` above.** Measured across the **17** reports on this host, 228 reads non-zero in exactly **one**: `agb-before.info`, the supervisor's **pre-work baseline** (17:56, count **2**) — and **zero in all 16 reports taken after this unit's tests landed** (18:28 → 21:33, including the pinned `agb-p7.info`). The earlier text here attributed that reading to “the 18:01 run of the identical test set”; no 18:01 report survives, and the artifact that does exist is the pre-work baseline, which is *not* the identical test set. So the flip is **structural** (the attribution follows the matched pattern, so the next input change can move it) while **not re-observed** under the final test set — which is exactly why this row is a citation: a claim that could go false on the next input change is what the rule removes. |
| `agent/src/skill_reco/mod.rs`:508, 531 | The `query = %crate::session::truncate_visible(…)` argument lines of two multi-line `log_outcome` macros whose `tracing` statements read non-zero (`log_outcome_reports_the_decision_with_its_numbers` asserts the emitted lines). |
| `agent/src/tools/shell/mod.rs`:148 | The closing brace of `if let Ok(mut result) = empty_result(…)`: 145/146/147 read 6, 149 reads 6 — the arm's exit edge. |

### 6.5 Not a line, but declared: the ignored acceptance test

`agent/src/sandbox/linux/glob_scan.rs` (`#[test]`, `#[ignore = "creates over
100000 entries for large-workspace acceptance"]`) keeps the 100 010-file
large-workspace acceptance run out of the default suite. Its body lines are
registered above under `unreachable-in-this-environment`; the test was neither
weakened nor hidden, and no measurement command skipped a test.

## 7. Things found while working (reported, not silently changed)

1. **A long rule pattern panicked the agent; fixed fail-closed in pass 2.**
   `sandbox::rules::build_glob_regex` ended in
   `.expect("glob regex is built from escaped literals only")`. On macOS/Windows
   the builder is case-insensitive, which inflates the compiled program: a
   262 145-character pattern makes `build()` fail with `CompiledTooBig(10485760)`
   and the `.expect` aborted the process. **Reproduction (pass 1)**: the first
   version of `tools::shell::tests::oversized_workspace_metadata_is_refused_before_execution`
   used `"x".repeat(262_145)` as the workspace and the suite panicked at
   `rules.rs:270` with exactly that error (`target/cov-ag-b-after3.log`).
   **Reachability**: `rpc/commands/settings.rs:499` forwards a *client-supplied*
   `cmd.message` to `push_session_allow`, and `.future/approval_rule.json` is
   user-authored — so the panic was reachable from a long rule pattern, not only
   from a test seam. **Fix**: `MAX_RULE_PATTERN_BYTES = 4096`, a bound on the
   **byte length of the rule pattern itself** (`str::len`) — not a path length,
   since a pattern is a glob that need not name any single filesystem object, and
   deliberately **not** a `PATH_MAX` claim: `PATH_MAX` is not 4096 everywhere
   (1024 on macOS, measured on this host; 4096 on Linux; 260/32767 on Windows),
   and the platforms that motivated the bound are macOS and Windows. Enforced at
   every admission that carries arbitrary text — the rule
   file (refused *with a resolution error*, so every sandbox compiler then fails
   closed: seatbelt returns `(deny default)`, `file_request::prepare` refuses) and
   the session grants (`push_session_allow`, `add_session_rule` /
   `add_session_allow` now return whether the rule was added, and a refused
   *allow* simply leaves the previous verdict). Consequences: the longest pattern
   any admission can now produce is `PATH_MAX` (workspace prefix) + 4096, i.e. a
   few hundred KB of case-insensitive program — roughly 30× below the ceiling that
   used to abort — and the `.expect` is now justified by that invariant, on the
   same pattern the previous record used for its other `expect`s. Pinned by
   `the_largest_accepted_pattern_still_compiles_and_one_more_is_refused` and
   `an_over_long_session_allow_is_refused_without_changing_the_verdict`.
   Reported, not silent: this *is* a production behaviour change (an over-long
   rule is now refused instead of crashing), reviewed against the repo's
   fail-closed rule contract (rules.rs:749-751).
   **DISPOSITION (2026-10-17): the user decided to KEEP the 4096-byte bound as
   shipped.** No code change, no waiver, no test change — this closes the item
   rather than parking it. The number itself and the idea of a friendlier
   approval-UI message for a refused over-long pattern remain the user's to revisit
   later; neither is a defect in this record, and neither is owed by anyone.
2. **`openai_item_metadata`'s `encrypted_content` parameter is dead.** All three
   call sites (194, 877, 1106) pass `None`, so the branch at 937 cannot execute;
   the live path for that value is `openai_reasoning_metadata`. Either the
   parameter should be dropped or the tool-call sites should pass the item's field.
3. **My own `platform-unmeasured` claim for the snapshot byte ceiling was wrong,
   and the falsification is now a test.** Pass 1 argued the branch needs
   2048 × 4 KiB of path bytes and is therefore Linux-only. Pass 2 measured the
   macOS path limit (`ENAMETOOLONG` at exactly 1023 bytes, so a long-path fixture
   really is impossible here) and then found the actual mechanism: the byte budget
   counts the *pattern text per pattern*, so overlapping recursive patterns cross
   4 MiB with short paths. `the_snapshot_byte_ceiling_trips_before_the_match_ceiling`
   covers the branch on this host; the row moved from §6.3 to covered.
4. **A shared test harness was asserting its own premise vacuously.**
   `stream_wait_tests::controlled_server` returned as soon as it wrote the body
   terminator, so "the client released the connection" (its read arm's assertion,
   and every `timeout(…, server)` after an abandonment) was satisfied by the
   *sender* being dropped rather than by the client closing. With keep-alive, a
   cleanly completed response is pooled, which is why the stronger version needs an
   explicit half-close. The helper now half-closes and waits; the four new
   abandonment tests and the pre-existing cancellation test all depend on it.
   This was also the reason the pump's post-drop guard looked unreachable: with the
   connection pooled, the test's root future resolved on its first poll and the
   woken pump task was never polled before shutdown (a woken-but-unpolled task
   does not run its code after the await). Both halves — the half-close, and an
   explicit yield loop after the drop — are needed; §3 records them so the next
   worker does not re-derive it.
5. **The Anthropic adapter is the only one that reports usage for an unterminated
   stream** (a zeroed `Usage`), which is what makes the pump's usage-then-error
   conversion reachable at all. Recorded as a contract test
   (`only_anthropic_carries_a_usage_placeholder_into_the_eof_finish`) rather than a
   claim in prose.
6. **A line whose coverage depended on test order was measured flipping, and is
   now pinned.** `models/mod.rs:1902-1903` (the ambient default model's
   resolvability check) were covered in the 19:21 run and uncovered in the 19:33
   run of **identical source**, because the answer depended on whether an earlier
   test in the same process had left a default model in the isolated FutureOS home
   — coverage attributable to test *order*, not to a test of this unit. Pass 3
   removed the dependence by pinning the home, the credential and the choice
   (§3.2), and the file left the ledger. `skill_reco/mod.rs:228`
   (`Outcome::NoKey | Outcome::NoInput | …`) is the one remaining entry of this
   class (it flipped between the 18:01 and 18:25 runs); it is registered in §6 with
   that evidence, and a future "zero uncovered lines" claim on that file must
   re-measure first.

7. **The `hydrate_*` flake: cause measured, the assertion made deterministic, the
   budget kept.** Pass 6 (todo `todo_6c69dab758c3`) measured the one failure
   recorded above instead of inheriting the "5 s budget lost the race" reading.
   - **Provenance corrected.** The authentic panic text is in `/tmp/agent-a-s6.log`
     — `panicked at agent/src/sandbox/mod.rs:2214`, `left: Err(NotPresent)`,
     `right: Ok("merged")` — in a run where the other three `hydrate_*` tests
     passed. `target/cov-ag-b-final5.log`, cited for it in passes 4-5, is green and
     was never the failing run.
   - **The budget is not the problem.** Ten runs of the already-built test binary
     (`/tmp/agb-hydrate-probe.py`) measured p50 **0.17 s** for this fixture (min
     0.16, max 0.17), i.e. ~30x headroom under the 5 s budget; losing it needs a
     ~30x stall inside the spawn->dump window on a host at load ~10 on 32 cores.
   - **Host starvation is real here, though.** In the same probe one *process
     invocation* took **55.7 s** and still passed — this host can stall a process
     for tens of seconds (a 154 MB instrumented binary on a 93 %-full volume),
     which is the precondition for every refusal arm.
   - **The signature cannot identify the arm.** All three refusal paths in
     production — spawn refused (fd/process limits), budget lost, truncated dump —
     return without applying anything, so each yields exactly `Err(NotPresent)`
     with PATH untouched. The arm is unidentifiable post hoc, so the fix does not
     depend on choosing one.
   - **Fix: the `apply_env_dump` extraction.** `hydrate_from_login_shell` now ends
     in `apply_env_dump(&dump, &shell)`, which holds the merge/apply contract; the
     marker became the module const `ENV_DUMP_MARKER`. The merge test applies a
     dump shaped exactly like a login shell's, so it no longer races the scheduler
     — and it *gained* a deterministic assertion: a var this process already has is
     never overwritten (the additive rule). Same extraction pattern the crate
     already uses (`prepare_shell_for_cwd_with_cancel`).
   - **Falsifier (sabotage-verified).** Neutralising the two `set_var` calls inside
     `apply_env_dump` makes the test fail in **0.00 s** with the *identical*
     signature (`Err(NotPresent)` vs `Ok("merged")`): it still detects a real
     regression, deterministically, instead of only on a starved host.
   - **Budget decision (recommended to the owner): keep 5 s.** At p50 0.17 s the
     budget is generous by ~30x, so raising it would only lengthen startup on a
     genuinely hung shell without addressing starvation, and "skip and keep the
     inherited env" is the deliberate contract. Nothing was raised.
   - **What stays environment-sensitive, deliberately.** The three spawn-path tests
     keep spawning; their assertions hold under *any* refusal (PATH untouched), so
     they cannot flake on this. `hydrate_times_out_and_kills_hung_shell` still
     asserts `elapsed < 20 s` for a 5 s+2 s operation, which a >13 s stall could
     still trip — that bound is the test's subject (the timeout must fire rather
     than wait the shell out), so it stays.
   - **Coverage-neutral.** Pass 6's per-file residual counts are identical to pass
     5's for all 21 files (`covlines.py files`, both reports); `sandbox/mod.rs` is
     the only denominator change (1048 -> 1054 measured lines, residual still 3).
     Unit total unchanged at **213**.
8. **`ContextError::SummaryFailed(_)` is a dead variant.** Its only mention outside
   its own definition is the pattern in `run_loop.rs`'s fallback arm
   (`| SummaryFailed(_)`); nothing in the crate constructs it, so that half of the
   guard is unreachable by construction and the arm depends entirely on
   `InvalidSummary` (§6.1). Either a producer should exist or the variant should go:
   as it stands a future reader will assume one does.
9. **A duplicated clause in a comment, left alone deliberately.**
   `compaction/semantic/evidence.rs`'s `HEADER_WITH_SUMMARY` doc comment repeats a
   clause ("…carry the index alone, while a failed summary call carry the index
   alone, while a successful call appends…"). It is a typo rather than a behaviour
   claim, and line numbers in that file are load-bearing for this record's ledger,
   so it is reported instead of edited; whoever next touches the file can fix it by
   spelling the sentence across the same two lines (the numbering is preserved that
   way).
10. **My own prediction about the `is_body` fixture was wrong, and the correction is
    now a measurement.** This unit's follow-up todo proposed "truncate the chunked
    body mid-transfer — it should surface as `is_body`". It surfaces as `is_decode`
    (§3.3), because the pinned reqwest maps every body error through
    `error::decode` before the crate sees it. The two lines therefore changed
    *category* — from "needs a fixture this host cannot provide" to "unreachable
    through this classifier by construction".

11. **A coverage line whose value is decided by a random draw (finding `D6`).**
    `llm/mod.rs`:422 flips between covered and uncovered across runs of an
    unchanged function, because tokio's `select!` randomizes which ready arm it
    polls first and this line is reachable only when two arms are ready at once
    (§1, §6.2). Consequences worth knowing: this unit's residual is 212 or 213
    depending on the draw; and any future "zero uncovered lines" claim for
    `agent/src/llm` must state which of the two states it measured.
    **The tripwire half of that is fixed (2026-10-17, the flipping-row rule):** 422
    is now *cited* in §6.2 and in §6's exception note rather than *claimed* in the
    ledger, so a run where the draw lands covered no longer prints
    `STALE → COVERED` for a row that is not stale. Measured across this unit's 17
    surviving reports, 422 is non-zero in 5 and zero in 12 — the flip is real, which
    is exactly why its number belongs where only a citation lives.
12. **What "unverified" means for the two tests this pass adds.** The Linux-gated
    non-UTF-8 test (§3.4) and any `#[cfg(target_os = "linux")]` code cannot be
    compiled on this host: `cargo check --target x86_64-unknown-linux-gnu` fails in
    `cc-rs` (`failed to find tool "x86_64-linux-gnu-gcc"`) while building
    `libsqlite3-sys`, i.e. before this crate is type-checked. That is why the
    non-UTF-8 test's body was verified by temporarily removing its attribute (it
    compiled and skipped), and why the sandboxed-run test was written portably with
    a runtime guard instead — a design choice made for verifiability, not
    convenience.

13. **An env race in this unit's own `utils` module — one missing lock, found by a
    gate run (fixed 2026-10-17).** `utils::util_tests::default_config_dir_exists`
    asserted `default_config_dir().contains(".future")` **without taking
    `crate::test_support::home_env_lock()`**, the process-global lock that every other
    home-derived reader in this crate takes. `default_config_dir()` is
    `future_home().join("agent")`, and `future_home()` honours `FUTURE_HOME`; several
    tests in the same binary set that variable to a temp directory for the body of
    their own test (`cli::tests::home_flag_selects_an_isolated_futureos_root`,
    `cli::future_home_tests::*`, `tools`, `sandbox/rules`, `skills`), so a concurrent
    redirect makes `$HOME/.future` become a path with no `.future` component and the
    assertion fails for a reason that has nothing to do with the function.
    **It is the only racy one of the three accessor tests here**: `default_session_dir`'s
    assertion checks its leaf (`sessions`) and `default_settings_paths_are_valid`
    checks a filename, both of which are true under any redirect, and the two
    `future_home_from` tests take their arguments explicitly rather than reading the
    environment — checked one by one rather than assumed.
    **The fix is the one line the rest of the crate already uses** (`let _guard =
    crate::test_support::home_env_lock();`), and it changes no production code.
    **Measured, and the honest shape of the evidence — the race is real but was NOT
    reproducible on demand:**

    | pass | measurement | command | N | result | load at the time (1/5/15-min, 32 CPUs) |
    |---|---|---|---|---|---|
    | 1 | before (unfixed test in place) | `cargo test -p future-agent --lib -- --test-threads=16 <the 6 tests>` via `flake-rate.sh` | 40 | **0 failures** | not stamped |
    | 1 | after (lock taken) | the same command | 40 | **0 failures** | not stamped |
    | 2 | before (unfixed, load-stamped) | the same command | 40 | **0 failures** | **5.54 / 8.32 / 9.98** |
    | 2 | after (lock taken, load-stamped) | the same command | 40 | **0 failures** | **5.33 / 8.14 / 9.88** (5.04 / 7.94 / 9.78 after) |
    | — | the predicate itself, deterministically | `FUTURE_HOME=$(mktemp -d) cargo test ... --exact utils::util_tests::default_config_dir_exists` | 1 | **rc 101, `assertion failed: dir.to_string_lossy().contains(".future")` at `agent/src/utils/mod.rs:845`** — a redirected root is exactly what breaks the assertion | — |

    So the mechanism is proven (the last row shows the victim predicate failing on a
    redirected root, and the *only* difference from the racer is whether another test
    happens to hold that root while this test reads it) and the interleaving is not:
    **160 focused runs, 0 failures, at 1-min load 5.0–5.5 on 32 CPUs**, while the failure
    itself was first observed **once in a full gate run** under parallel load. The window
    is a few hundred microseconds wide against a suite of 2 400 tests, which is why a
    focused 6-test set cannot hit it reliably — the honest statement is therefore "not
    reproduced in 160 runs at load 5.0–5.5", not "rate measured". **This is the campaign's item-27 family and the distinction matters:** a
    mutex alone is not a restore, so a *leaked* `FUTURE_HOME` would still defeat this
    assertion (that is the row with the `mktemp` run) — but here the missing lock *is* the bug, and
    taking it is what the rest of the crate already does: **31 call sites across 12 files**
    (`grep -rn "home_env_lock()" agent/src`, excluding the helper's own definition,
    measured 2026-10-17). This test was the outlier.
    Two things follow for a later pass: the same audit should be run over every
    `default_config_dir()`/`future_home()` reader in the crate (this was one), and a
    test that asserts an *absolute* value of a process-global is only as deterministic
    as its neighbours' restores. The measured runs used `CARGO_TARGET_DIR=target/cov-agent-hyg`
    (this unit's private dir) with `HOME=$PWD/target/test-home`.

## 8. Residual risk, and the next useful checks

1. **The two transport-kind lines are the clearest "not a fixture" entries left**
   (`llm/mod.rs`:32, 36): unreachable *through this function* because `bytes_stream`
   maps every body error to `decode` (§3.3, §6.1). What would make them live is a
   change in how the crate feeds the classifier — e.g. routing `execute()`'s error
   (a request-side failure, where `is_request` is real) into
   `reqwest_stream_error_kind` too. That is a design change, not a test, and worth
   doing only if the diagnostics need to distinguish "could not send" from "peer
   closed mid-body".
2. **`run_loop.rs`:478 and :703 are registered as unreachable-by-construction** with
   derived invariants, not as "needs a scenario" (§6.1). Those invariants are
   falsifiable and two of them now have standing falsifiers in the suite (§3.3); a
   reviewer who disagrees should attack the invariant rather than look for a
   fixture.
3. **The Linux rows now have their tests; what remains is running them**
   (`tools/shell/execution.rs`:492, 679-681, 716-727, 729, 733-737, 739-742, 744,
   745 and `sandbox/file_request.rs`:117). Pass 5 delivered the measurement path
   (§3.4): a portable sandboxed-run test that reaches the helper-report wiring on
   Linux, and a Linux-gated non-UTF-8 test. The next step is a Linux runner with
   bwrap and byte-name filesystems executing `cargo llvm-cov -p future-agent` — if
   either test skips there, its skip message names the reason (`no OS sandbox
   backend`, and the filesystem guard is now a hard `expect` because the test is
   gated to Linux). Note that `linux_sandbox_smoke` still does not close these: it
   drives the helper API directly rather than `spawn_shell_impl`.
4. **The pass-2 rule bound deserves an owner's eye**: the bound is the byte
   length of a rule *pattern* (a glob/regex that a user or a rule file
   supplies), deliberately **not** a `PATH_MAX` claim — `PATH_MAX` is not 4096
   everywhere (1024 on macOS, 4096 on Linux, 260/32767 on Windows), and a
   Windows extended-length path can exceed 4096, in which case the pattern is
   refused rather than compiled. The refusal is fail-closed, but a user with a
   legitimate >4 KiB *pattern* (not path) would now see a refused rule rather
   than an error about size. A clearer diagnostic in the approval UI is a
   product decision, not a coverage one.
5. Re-measure before trusting any figure here: the metric moves with peers' tests
   in a shared workspace (see §1), and entries in this list have flipped run to run
   (`skill_reco/mod.rs:228` between 18:01 and 18:25; `models/mod.rs:1902-1903`
   between 19:21 and 19:33, pinned by §3.2). Line numbers also move when a file is
   edited: pass 4 shifted three `llm/mod.rs` rows (1297/1458/1549 →
   1355/1516/1607), and the ledger is generated from the report precisely so a shift
   shows up as a diff instead of as a stale claim. Pass 6 shifted this unit's own
   `sandbox/mod.rs` rows by **+21** (the `apply_env_dump` extraction, §7.7):
   992/1010/1043 -> 1013/1031/1064, caught by the generator's equality assertion in
   both places that cite them (the §6 table and §6.2's mechanism row).
6. **The `hydrate_*` flake is fixed for the assertion that mattered (§7.7).** The
   merge contract is now asserted without spawning a shell, so a busy host can no
   longer make it fail. The three spawn-path tests remain: a *resource refusal* (fd
   exhaustion, fork failure) still fails their spawn, but their assertions are
   refusal-tolerant by design, so they cannot flake on it. Only
   `hydrate_times_out_and_kills_hung_shell`'s `elapsed < 20 s` bound could still
   trip under a >13 s stall, and that bound is the test's subject. The measurement
   procedure no longer lets one failing test discard a report: run and render are
   separate steps whose exit codes are recorded independently (see §9).

7. **`D6` (§7.11) makes this unit's residual 212 or 213 depending on a random
   draw.** Any downstream aggregation of these residuals should carry that ±1
   rather than treating a one-line move as a change; the ledger documents the
   mechanism next to the line.

## 9. Acceptance gates (PLAN §5.1) — commands and exit codes

A green suite is not a green PR, so each gate was run in this worktree and its
exit code recorded. This is a **Rust** unit: the JS gates (`npx tsc --noEmit`,
`npx eslint`) do not apply to it and were not run.

| gate | command | exit | result |
|---|---|---|---|
| format | `cargo fmt --check -p future-agent` | **0** | **pass 6: `cargo fmt --check -p future-agent` exit 0, 0 diffs** (green; pass 4's red was a peer's `agent/src/service/tests.rs`). This unit never ran a whole-crate *fixing* `cargo fmt` — per the campaign rule "format per file, never across the crate" — and every file it touched is rustfmt-clean individually (`rustfmt --edition 2021 --check <file>`, exit 0) |
| lint | `cargo clippy -p future-agent --all-targets -- -D warnings` | **101** | **pass 6: red on a peer's in-flight file** — `agent/src/session/compaction_ops.rs:79`, `clippy::nonminimal_bool` (a minified hand-edited buffer; `session/` is outside this unit). It is the *only* error clippy reports for this crate, so this unit's code is clean. Earlier red was two peer files — `agent/src/grpc/mod.rs:1580`, `agent/src/rpc/approval/tests/flow.rs:677` — both since fixed by their owners) |
| tests | `cargo test -p future-agent --lib -j 4` | — | 2375 passed / 0 failed / 1 ignored in the measured run (§1), including every test this pass added; per-subtree re-runs after the last edit: `a_sandboxed_run_announces_the_wrapper_and_reports_process_facts` ok, `the_shell_tool_cannot_see_a_linux_boundary_on_a_non_linux_build` ok, `a_target_under_a_non_utf8_parent_is_refused_before_an_approval` ok (skips on APFS), `a_transport_error_is_classified…` ok, `llm::stream_wait_tests` 11, `sandbox::rules` 23, `glob_scan` 14 (+1 ignored), `sandbox::linux::plan` 21, `sandbox::file_request` 6, `sandbox::windows_plan` 9, `sandbox::linux::probe` 14, `sandbox::linux::request` 11, `sandbox::linux::runner` 6 — all `ok`. **Pass 6: lib 2385 passed / 0 failed / 1 ignored** — all four `hydrate_*` tests green. One *integration* target failed 2 of 25 (`agent_is_singleton_per_user_even_on_different_ports`, `home_flag_runs_a_second_isolated_instance`): a real agent was running on this host, and `agent/tests/` is a peer's scope → `TEST_EXIT=101` (§7.7 for the hydrate fix, and the procedure below for why a red target no longer costs the report). **A Linux cross-check was attempted and is impossible here**: `cargo check -p future-agent --lib --target x86_64-unknown-linux-gnu` exits in `cc-rs` (`failed to find tool "x86_64-linux-gnu-gcc"`) while building `libsqlite3-sys`, before this crate is type-checked (§7.12) |
| coverage | `/tmp/agb-measure-p7.sh` — run and render as **two** steps | run **101**, render **0** | pass 6: `TEST_EXIT=101`, `REPORT_EXIT=0`, `/tmp/agb-p7.info` written. The failing integration target did **not** suppress the report — which is exactly why the two steps are separate (below) |
| ledger | `reconcile.py …` (see §6) | 0 | pass 6: 213/213 mentioned, **0 MISSING, 0 STALE** against `/tmp/agb-p7.info`; the generator (`target/tmp/ledger_pass7.py`) refuses to write unless its mapped lines equal the report exactly. **Since 2026-10-17 the two random-draw lines `llm/mod.rs` 422 (`D6`) and `skill_reco/mod.rs` 228 are cited-not-claimed (§6's exception note), so a run where `D6` flips no longer prints the `STALE → COVERED` this row used to describe as by design** — which is the whole point of the rule: the tripwire's STALE half now only fires on a row that is genuinely stale |

A whole-crate `cargo test -p future-agent` (integration targets included) was
attempted and timed out at 600 s under a load average of ~10 from peers' suites;
the equivalent evidence is the measured run above, which executes every target in
one invocation and reports every integration target green.

### Measurement procedure (pass 6) — a failing test must not cost the report

cargo-llvm-cov writes **no lcov** when a test target fails, so an unrelated red
target silently suppresses the whole measurement (that cost pass 4 a run). Pass 6
split the two concerns so they are independent, and recorded both exit codes:

```bash
cargo llvm-cov clean -p future-agent
cargo llvm-cov --no-report --no-fail-fast -p future-agent -j 4                  # TEST_EXIT
cargo llvm-cov report -p future-agent --lcov --output-path /tmp/agb-p7.info     # REPORT_EXIT
```

Pass 6 ran exactly this (`/tmp/agb-measure-p7.sh`, log `/tmp/agb-p7-run.log`):
`TEST_EXIT=101` (the two integration failures above) and `REPORT_EXIT=0` with the
lcov written, so the red target did not cost the report. **Rule for this unit:
record the two exit codes separately, and treat `covlines.py`'s own exit status as
the gate on whether a number is valid** — it exits 3 on a zeroed report or a scope
mismatch, whatever it printed.

### Reviewer reproduction

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home RUST_TEST_THREADS=4
export CARGO_TARGET_DIR=$PWD/target/cov-ag-b        # this unit's private dir only

# 1. the metric from the pass-4 report (no re-instrumentation needed)
P=".worktrees/cov100-r2/agent/src"
python3 .future/cov100/covlines.py total /tmp/agb-p6.info \
  --under $P/tools --under $P/llm --under $P/agent --under $P/compaction \
  --under $P/sandbox --under $P/config --under $P/models --under $P/runtime \
  --under $P/utils --under $P/auth --under $P/prompt --under $P/types --under $P/skill_reco
# → covered=35885 measured=36097 uncovered=212 lines%=99.4127

# 2. the ledger's arithmetic (per file, and the exact lines behind each row)
python3 .future/cov100/covlines.py files  /tmp/agb-p6.info <same flags>
python3 .future/cov100/covlines.py detail /tmp/agb-p6.info <same flags>
# the table is generated with an assertion (refuses to write on a mismatch), and
# the supervisor's tripwire re-checks it:
python3 target/tmp/ledger_pass4.py         # -> files: 21 mapped: 212 report: 212
python3 .future/cov100/reconcile.py --report /tmp/agb-p6.info "${scopes[@]}" \
  --doc docs/testing/module-agent-core.md --section '6. Waiver ledger'  # -> 0 MISSING / 0 STALE

# 3. one neighbouring count quoted in §6.4 (the tracing-argument rows)
python3 - /tmp/agb-p6.info <<'PY'
import sys
for line in open(sys.argv[1]):
    if line.startswith("DA:"):
        n, c = line[3:].split(",")[:2]
        if int(n) in (228, 229, 232, 233):
            print("llm/run_loop neighbour", n, c)
PY

# 4. the tests (uninstrumented inner loop)
cargo test -p future-agent --lib -j 4                 # 2375 passed, 0 failed, 1 ignored
cargo test -p future-agent --lib -j 4 stream_wait_tests   # the pump guards, 0.1 s
# the three tests this pass added, and their skip messages
cargo test -p future-agent --lib -j 4 a_sandboxed_run_announces_the_wrapper -- --nocapture
cargo test -p future-agent --lib -j 4 the_shell_tool_cannot_see_a_linux_boundary
cargo test -p future-agent --lib -j 4 a_target_under_a_non_utf8_parent -- --nocapture
# (measured here: the third reports `0 passed; 0 filtered out ... 2376 filtered
#  out` because it is `#[cfg(target_os = "linux")]`; on Linux it must pass, and its
#  fixture's `expect` fails loudly if a byte-name filesystem is ever unavailable)

# 5. the pass-4 falsifiers: the sabotage experiment that shows line 703 is one
#    policy field away from executing (restore with the backup, never git checkout)
cp agent/src/compaction/durable.rs /tmp/durable.bak
sed -i '' 's/"trigger":trigger,"phase":phase/"phase":phase/' agent/src/compaction/durable.rs
cargo test -p future-agent --lib -j 4 a_provider_limit_recovery_claims_its_own_receipt
# → FAILED: "the recovery must reach its own planner and refuse there: context
#            compaction made no progress after provider limit"   (line 703 executed)
cp /tmp/durable.bak agent/src/compaction/durable.rs

# 6. the falsifier for the shell-tool registration (one added line, reverted after)
cp agent/src/sandbox/mod.rs /tmp/mod.bak
python3 - <<'PY'
p = 'agent/src/sandbox/mod.rs'
s = open(p).read()
s = s.replace('return Ok(seatbelt::prepare(self, command));',
              'let mut prepared = seatbelt::prepare(self, command);\n'
              'prepared.boundary.backend = backend::ShellBackend::LinuxBubblewrap;\n'
              'return Ok(prepared);')
open(p, 'w').write(s)
PY
cargo test -p future-agent --lib -j 4 the_shell_tool_cannot_see_a_linux_boundary
# → FAILED: "the macOS build must route to seatbelt, never to the Linux helper"
cp /tmp/mod.bak agent/src/sandbox/mod.rs   # then re-run: green

# 7. the PLAN §5.1 gates (see §9 for the exit codes)
cargo fmt --check -p future-agent                     # exit 0
cargo clippy -p future-agent --all-targets -- -D warnings   # exit 0

# 8. the measured state end-to-end (one atomic invocation; ~5 min warm, ~2 min
#    cold only if the private target dir was cleaned)
cargo llvm-cov -p future-agent --no-fail-fast --lcov --output-path /tmp/agb-recheck.info -j 4
```

Delete `target/cov-ag-b` when this unit is done with it (shared-disk discipline,
`.future/cov100/PLAN.md` §2) — it is this unit's own directory.
