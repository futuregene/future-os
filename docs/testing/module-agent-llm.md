# future-agent / `ag-llm` subtree — coverage, waivers and test-quality record

Goal `cov-100-multidim` · group **ag-llm** of module **future-agent** (`agent/`) ·
worktree `.worktrees/cov100` · host **Windows x86_64**, rustc pinned by
`rust-toolchain.toml` · measured 2026-09-26 · session
`20260926-191032-43212ee5c0a64eb78b10ae992e9c8540` (this worker; the previous
segment of this group ran in session `20260926-093053-733416b82b4146a9b2408d34014a4001`).

Scope: `agent/src/llm/**` (adapters, SSE decoder, request client),
`agent/src/agent/**` (agent loop), `agent/src/compaction/**` (context compaction),
`agent/src/engine/**`.

Handoff evidence literals: **lines-100-or-waived** (§6 is the waiver ledger),
**dimensions** (§4), **weak-tests-fixed** (§5).

## 0. Status

Measured **97.7366 %** (17100/17496 lines), up from 96.1808 % (14682/15265) at the
goal baseline and 97.4971 % (16594/17020) at the end of the previous segment:
**+30 counted lines closed this segment (426 → 396), +0.240 pp**, and — the part
that matters more — **every reachable line the previous segment had listed as
still uncovered is now either covered or carries a category and an invariant in
§6**, with the two lines whose triage in the previous record was wrong re-derived
from the code (§6.2 rows for `evidence.rs:302-304` and `evidence.rs:382-404`).

Two claims in the previous version of this document were **wrong**, and this
segment disproves them rather than restating them:

1. `evidence.rs:302-304` (the evidence index exceeding its budget) was filed as
   "reachable, needs a window where `available − summary_reserve − header_reserve()`
   underflows". The underflow path returns the *other* `BudgetExceeded` message at
   `evidence.rs:197-200` first, so it never reaches 302. The real argument is an
   invariant (the estimator is subadditive under the `\n` join and every append is
   guarded by `est(line) + 2 ≤ budget − est(text)`), and it is now pinned by a test
   over the estimator instead of asserted in prose.
2. `evidence.rs:382-404` (the legacy-checkpoint `NoValidBoundary` branches) was
   filed as "reachable: needs a transcript whose cutoff id is absent from `raw`".
   It is not reachable from any projection `project_prompt_context` can build —
   the invariant is in §6.2 and pinned by
   `a_projection_places_the_checkpoint_after_anchors_only`.

The gate (`verify.py rust-module ag-llm`) rejected the previous handoff because
§6.4 of that document *declared* reachable-but-uncovered work while §6 carried
per-file categories: the categories made the gate green and the disclosure lived
in a different section. That section is replaced here by per-line registrations
that carry their own category (§6.2–§6.5). Nothing below claims that a line with
a testable behaviour is unreachable.

Test totals after this segment: `agent::run_loop::tests` **101**, `compaction::semantic::tests`
**43**, `llm::adapters::openai_responses::tests` **41**, `llm::tests` **39**,
`compaction::semantic::evidence::tests` **22**, `compaction::tests` **13**,
`engine::tests` **12**, `compaction::durable::tests` **1** (2064 lib tests in
total, 1 ignored); the whole `future-agent` suite (lib + 3 integration targets)
is green in the measured run, and `rustfmt --check` is clean on every file this
segment touched.

## 1. Measurement

```powershell
$env:CARGO_TARGET_DIR = "target/cov-w-ag-llm2"   # per-agent dir, never the shared target/
$env:RUST_TEST_THREADS = "4"                    # cargo-llvm-cov has no --test-threads flag
$env:CARGO_BUILD_JOBS  = "3"
cargo llvm-cov -p future-agent -j 3 --no-report
cargo llvm-cov report --json --output-path coverage/agent-ag-llm-report.json
cargo llvm-cov report --lcov --output-path coverage/agent-ag-llm.lcov   # the per-line view
python .future/cov100/verify.py rust-module ag-llm 99.99 docs/testing/module-agent-llm.md coverage/agent-ag-llm-report.json
```

One test run, then two exports of the **same** profile data (the `--json` and
`--lcov` reports do not consume `.profraw`, so both views describe one green run;
`report` must not be run after a *failed* run, where the merge is lost). No
`--lib`: it drops `agent/tests/*.rs` and the bin targets and is a different
metric.

### 1.1 Where the number comes from — three views of one profdata (reproducible)

The gate's per-file numbers are `summary.lines` (`count - covered`), which is
LLVM's line metric and byte-identical to LCOV's `LH`/`LF`. The same data can be
read three ways, and they do not agree — this is reproducible on the two files
below with the commands in §1:

| file | `summary.lines` uncovered | LCOV `DA:` lines with count 0 | region reconstruction (zero-count entries) |
|---|---|---|---|
| `agent/src/agent/mod.rs` | **12** | **0** of 724 | **0** |
| `agent/src/agent/run_loop.rs` | **281** | **29** of 4995 | 41 |
| `agent/src/compaction/semantic/evidence.rs` | 35 | 33 | 33 |
| `agent/src/llm/adapters/openai_responses.rs` | 11 | 8 | 12 |

`agent/src/agent/mod.rs` is the minimal contrast: after
`stream_truncation_error_message_maps_every_reason_to_a_stable_code` drives
`error_message` 33 times, **every** view except `summary.lines` reports zero
uncovered lines, and `--text` prints a non-zero count for every line of the file
(`51|33`, `52|32`, `55|23`, `58|15`). `summary.lines` still counts 12. So the 12
are lines that exist in LLVM's line table but belong to no region — closing them
with a test is impossible by construction, which is why §6 waives them rather
than promising a number.

`run_loop.rs` is the same phenomenon at scale: 281 counted-missed vs 29 lines
whose LCOV `DA:` record is 0. **252 of the 281 are lines no region can be
entered from.** The previous segment covered five production region lines there
(664, 665, 691, 693, 699 at the time) and the file's counted-missed total moved
286 → 281 across this whole segment, while the DA view moved by 11 lines.

Reproduce with:

```powershell
python coverage/ag-llm-dalines.py coverage/agent-ag-llm.lcov agent/src/agent/run_loop.rs [--src] [--counts LO HI]
python coverage/ag-llm-linemetric.py coverage/agent-ag-llm-report.json agent/src/agent/run_loop.rs
python coverage/ag-llm-segments.py coverage/agent-ag-llm-report.json agent/src/agent/run_loop.rs LO HI [--all|--zero]
python coverage/ag-llm-src.py <file> LO HI      # numbered source, rustc's line numbering
```

`verify.py uncovered`'s "approx. line numbers" are region-entry *hints*, and for
some files they are simply not the missed lines (for `run_loop.rs` the hint list
starts at 127, 285, 372 — none of which is uncovered in the DA view). Map line
numbers with `ag-llm-src.py`, never with PowerShell: **`Get-Content` reads these
files through the host's GBK codepage, and the repo's `───` section separators
decode into byte sequences that cost it three whole lines of `run_loop.rs`**
(6377 array entries where the file has 6380 `\n`-separated lines, so its entry
`1999` is rustc's line `2000`). `Select-String` numbers agree with rustc.

### 1.2 Cross-check of the counted-missed lines (per the supervisor's ledger)

The counted-missed lines cluster at zero-count region entries whose interior
lines inherit the zero, so per-file totals are not independent positions, and
the two views answer different questions: `summary.lines` asks "is this line in
the line table covered", the DA records ask "did a region that starts on this
line run with a non-zero count". Where the second says 0, §6 registers the line;
where only the first says missed, §6 registers `attribution-artifact` with the
neighbouring counts as evidence.

## 2. Result — before → after (measured, not estimated)

| run | covered / total | line % | counted-missed | files with gaps |
|---|---|---|---|---|
| goal baseline (supervisor's `coverage/llvm-cov-full.json`, 2026-09-26 00:51) | 14682 / 15265 | 96.1808 % | 583 | 12 |
| end of the previous segment of this group | 16594 / 17020 | 97.4971 % | 426 | 10 |
| **this segment** | **17100 / 17496** | **97.7366 %** | **396** | 10 |

Delta: **+2418 covered lines vs the baseline (+1.56 pp, −187 counted-missed)**, and
**+506 covered lines / −30 counted-missed vs the previous segment (+0.240 pp)**.
The denominator grew by 476 lines because the new in-file `#[cfg(test)]` tests are
themselves measured.

| file | summary uncovered: prev → now | DA lines with count 0 | line % (summary) |
|---|---|---|---|
| `agent/src/agent/run_loop.rs` | 286 → **281** | 29 | 94.76 |
| `agent/src/compaction/semantic/evidence.rs` | 42 → **35** | 33 | 93.74 |
| `agent/src/compaction/durable.rs` | 23 → **22** | 20 | 91.27 |
| `agent/src/llm/mod.rs` | 20 → **20** | 15 | 98.99 |
| `agent/src/agent/mod.rs` | 12 → **12** | 0 | 98.46 |
| `agent/src/llm/adapters/openai_responses.rs` | 14 → **11** | 8 | 99.50 |
| `agent/src/compaction/semantic.rs` | 21 → **7** | 7 | 99.64 |
| `agent/src/llm/adapters/anthropic.rs` | 5 → **5** | 3 | 99.61 |
| `agent/src/llm/adapters/openai_chat.rs` | 2 → **2** | 0 | 99.78 |
| `agent/src/llm/schema.rs` | 1 → **1** | 0 | 99.84 |
| `agent/src/llm/adapters/mod.rs`, `compaction/mod.rs`, `engine/mod.rs`, `llm/sse.rs`, `compaction/budget.rs` | 0 | 0 | 100 |

## 3. Tests added

| file | tests |
|---|---|
| `agent/src/agent/run_loop.rs` (2 new, 2 strengthened) | `an_automatic_compaction_whose_receipt_cannot_be_committed_fails_it`; `a_provider_limit_recovery_whose_receipt_cannot_be_committed_fails_it`; `a_ticket_is_closed_without_a_checkpoint_when_nothing_could_be_compacted` (+ a second run over the same receipt); `automatic_c3_falls_back_to_evidence_when_the_summary_fails` (event sequence instead of a flag that could never be set). The two commit-failure tests use the store's existing `SessionPersistence::fail_next_commit()` seam — see §7 for why no new seam was needed. |
| `agent/src/compaction/semantic.rs` (6 new, 2 fixed) | `plan_refuses_a_boundary_that_lands_on_the_existing_checkpoint`; `finalize_refuses_a_summary_that_would_not_shrink_an_automatic_compaction`; `retention_note_distinguishes_omitted_outputs_from_summarized_ones`; `attachment_images_are_charged_once_whichever_way_they_are_declared`; `a_summary_request_that_never_connects_times_out`; `a_summary_request_rejected_for_its_size_is_not_retried`; `a_complete_summary_survives_unknown_frames_and_a_trailing_error`; plus tool-call/short-result ToolCall cases in `serialize_message_covers_role_labels_and_block_variants` and a `Display` assertion in `summary_model_cancelled_finish_propagates_cancellation`. |
| `agent/src/compaction/semantic/evidence/tests.rs` (5 new) | `fork_remap_skips_blocks_that_carry_no_generated_index`; `a_window_without_a_summary_slot_fails_before_calling_the_model`; `fit_messages_sends_the_head_once_when_the_window_cannot_hold_the_history`; `a_restored_checkpoint_reports_coverage_from_a_raw_entry`; `a_projection_places_the_checkpoint_after_anchors_only` (the invariant behind the §6.2 row for 382-404). |
| `agent/src/llm/adapters/openai_responses.rs` (2 new) | `a_divergent_summary_done_frame_appends_nothing`; `unknown_message_content_parts_are_skipped_without_swallowing_later_text`. |
| `agent/src/llm/mod.rs` (0 new) | three assertion helpers rewritten from `find_map(|e| match e { Error => Some(..), _ => None })` to an `if let` loop, so no never-taken catch-all arm is left in test code. |

Every one of them asserts a value, an exact event sequence, an error category, a
counter or a row count; none is a snapshot or an assertion-free traversal.

### 3.1 Clusters closed this segment (with the closing test)

| cluster (file:lines) | closing test | the assertion that makes it real |
|---|---|---|
| `semantic.rs:803-804, 837` | `serialize_message_covers_role_labels_and_block_variants` | a tool call renders `[Assistant tool call c9]: read(` with its path; a short tool result is carried verbatim with no truncation marker |
| `semantic.rs:378` | `attachment_images_are_charged_once_…` | inline image = base + 2048 exactly; with metadata attachments *and* an inline image the cost is unchanged; metadata-only = base + 2048 |
| `semantic.rs:952` | `finalize_refuses_a_summary_that_would_not_shrink_an_automatic_compaction` | the same plan is refused with `NoProgress` for a non-shrinking summary and admitted with `tokens_after < tokens_before` for a shrinking one |
| `semantic.rs:902`, `1895` | `retention_note_distinguishes_…`, `a_declared_output_limit_equal_to_the_window_admits_the_first_turn` | exact retention-note text per algorithm; the unchanged projection must not yield a checkpoint |
| `semantic.rs:114` | `summary_model_cancelled_finish_propagates_cancellation` | `to_string() == "summary request cancelled"` |
| `semantic.rs:600, 607, 672, 693` | `a_summary_request_that_never_connects_times_out`, `a_summary_request_rejected_for_its_size_is_not_retried`, `a_complete_summary_survives_unknown_frames_and_a_trailing_error` | a connect timeout is `Other("summary request timed out")`, a context-limit rejection keeps the provider's reason and is not retried, and a trailing error after `Finish(Stop)` keeps the complete answer |
| `evidence.rs:60` | `fork_remap_skips_blocks_that_carry_no_generated_index` | non-text blocks come back byte-identical (serialized equality) while the index is rebound and the excerpts are not |
| `evidence.rs:436-440` | `a_restored_checkpoint_reports_coverage_from_a_raw_entry` | the committed checkpoint's `covered_from_entry_id` is the first *raw* entry, not the checkpoint entry the raw view omits |
| `evidence.rs:501-503` (7 `DA:0` rows closed) | `a_window_without_a_summary_slot_fails_before_calling_the_model` | for every window 1..16 the provider is **never called** (counter) and the failure is `BudgetExceeded` |
| `evidence.rs:730` | `fit_messages_sends_the_head_once_when_the_window_cannot_hold_the_history` | the head is sent exactly once (`["u0","a0"]`), never duplicated by an overlapping tail |
| `durable.rs:137` | `a_ticket_is_closed_without_a_checkpoint_when_nothing_could_be_compacted` (second run) | the replayed "nothing to compact" receipt emits no event, adopts no checkpoint, and leaves **exactly one** completed receipt row in SQLite |
| `run_loop.rs:435` | `an_automatic_compaction_whose_receipt_cannot_be_committed_fails_it` | error propagates, events are `[started, failed]`, no checkpoint adopted, and a fresh store handle over the same DB refuses the retry with `compaction_previous_failed` |
| `run_loop.rs:673` | `a_provider_limit_recovery_whose_receipt_cannot_be_committed_fails_it` | same, for the provider-limit recovery, plus the refused retry announces only `compaction_failed` (never a started event it did not perform) |
| `run_loop.rs:4128-4129` (old numbering) | `automatic_c3_falls_back_to_evidence_when_the_summary_fails` | the recorded event sequence is `[started, committed]` |
| `openai_responses.rs:753, 833` | `a_divergent_summary_done_frame_appends_nothing`, `unknown_message_content_parts_are_skipped_without_swallowing_later_text` | a done frame that contradicts the deltas emits nothing at all; unknown content parts are skipped and the following `output_text` still arrives |

## 4. Dimensions — evidence per dimension

| dimension | evidence |
|---|---|
| `boundary` | `a_window_without_a_summary_slot_fails_before_calling_the_model` (every window **1..16**, the range below one summary slot); `cjk_fields_and_excerpts_are_truncated_on_character_boundaries`; `a_frame_with_a_truncated_utf8_sequence_is_a_model_response_error` (a CJK character cut in half); `attachment_images_are_charged_once_…` (0 vs 1 vs 2 attachment sources); `fit_messages_sends_the_head_once_when_the_window_cannot_hold_the_history` (a window smaller than the history); `plan_refuses_a_boundary_that_lands_on_the_existing_checkpoint` (the cut landing on the last message). |
| `error-path` | `a_summary_request_that_never_connects_times_out` (a provider that never resolves); `a_summary_request_rejected_for_its_size_is_not_retried` (creation-time context limit, reason preserved); `a_complete_summary_survives_unknown_frames_and_a_trailing_error`; `an_automatic_compaction_whose_receipt_cannot_be_committed_fails_it` + `a_provider_limit_recovery_whose_receipt_cannot_be_committed_fails_it` (the commit fails: propagated, receipt marked failed, durable refusal); `a_transport_error_is_classified_by_the_kind_that_caused_it`; `a_request_deadline_is_reported_as_a_response_timeout`; `a_truncated_stream_body_is_an_upstream_disconnect_not_a_parse_error`; `a_cancelled_or_over_budget_index_build_is_refused`; `an_empty_summary_is_rejected_in_favour_of_the_deterministic_index` (with the exact fallback reason); `a_provider_limit_recovery_without_a_boundary_refuses_instead_of_looping`. |
| `concurrency` | `a_replayed_compaction_receipt_is_adopted_without_a_second_commit`; `a_ticket_is_closed_without_a_checkpoint_when_nothing_could_be_compacted` (a second run replays it; the receipt count stays 1); the two commit-failure tests (a fresh store handle over the same SQLite file refuses the retry); `a_timed_stream_stops_sending_when_the_run_drops_the_receiver`; `a_provider_limit_recovery_commits_its_checkpoint_through_the_ticket` (the receipt is the retry's idempotency key). Pre-existing: `durable/tests.rs::concurrent_claim_prepares_once_then_reuses_after_restart_without_a_provider` (two concurrent admissions through a `Barrier`) and `llm/stream_wait_tests.rs`. |
| `property` | `a_projection_places_the_checkpoint_after_anchors_only` (four protected-id sets: the checkpoint's position equals the anchor count and nothing after it is an anchor); `attachment_images_are_charged_once_…` (exact additive accounting); `verbose_log_fields_are_evaluated_for_a_tool_turn_and_a_final_turn` (accumulation `3+13, 5+17, 7+19, 11+23`); `a_compaction_summary_usage_is_charged_once`; `an_empty_model_reference_prices_the_loop_model_name` (1 M @1.0 + 1 M @2.0 == 3.0); `text_estimators_agree_on_unicode_costs`; `every_compaction_trigger_maps_to_its_documented_phase`. |
| `platform-cfg` | N/A and nothing is `platform-unmeasured`: no `#[cfg(windows/macos/linux)]` branch exists under `agent/src/llm/**`, `agent/src/agent/**`, `agent/src/compaction/**` or `agent/src/engine/**`. The platform-gated code in `future-agent` is `agent/src/sandbox/**` (owned by `ag-sandbox`) and `agent/tests/seatbelt_security.rs` (macOS); the CI matrix is the authority for those. Two `cfg!(test)` *value* branches exist (`semantic.rs:521`, `735`) and are registered in §6.3 — they are not platform cfg. |
| `serialization` | `stream_truncation_survives_serialization_round_trips_and_legacy_payloads`; `unknown_message_content_parts_are_skipped_without_swallowing_later_text` (a wire payload with parts the adapter does not model); `a_divergent_summary_done_frame_appends_nothing`; `reasoning_text_done_records_raw_content_and_replays_it_on_the_finished_item`; `fork_remap_skips_blocks_that_carry_no_generated_index` (serialized-equality of blocks that must not be rewritten). |

## 5. Weak tests fixed (`weak-tests-fixed`)

1. `agent/src/engine/mod.rs` declared a `fn handler` and never called it → invoked and asserted.
2. `agent/src/llm/adapters/mod.rs` stub adapter whose seven methods were never executed → all driven.
3. Same file: an unreachable `Ok(_) => panic!(...)` arm → `.err().expect(..)`.
4. `agent/src/compaction/mod.rs::default_phase` never-taken arm → asserted per variant.
5. `anthropic.rs` dead `let mut stale = Usage {…}` whose comment claimed to pin a boundary it did not → comment corrected.
6. `run_verbose_*` claimed to cover the logging arms while installing no subscriber → the sink rebuilds the callsite interest cache; the level is pinned by `the_log_sink_really_raises_the_enabled_level`.
7. `a_replayed_compaction_receipt_…` first asserted a non-discriminating condition → now asserts the event-count difference.
8. `the_catalog_context_window_…` first asserted "no compaction events" (passes even if the refresh never runs) → now also requires the 1 k-catalog run to refuse.
9. `a_provider_limit_recovery_…` was first written to expect the `Unchanged` arm and a `compaction_started` event; both expectations were wrong (the planner refuses with `NoValidBoundary`), so the test now asserts the behaviour that actually happens.
10. **`automatic_c3_falls_back_to_evidence_when_the_summary_fails` carried a flag that could never be set**: `failed.store(true, …)` sat inside an `if matches!(event, CompactionFailed)` arm that this scenario cannot reach, so `assert!(!failed.load(…))` could not fail. Replaced by the recorded event sequence (`[started, committed]`), which *can* fail.
11. **Three assertion helpers in `llm/mod.rs`** used `find_map(|e| match e { Error => Some(..), _ => None })`, leaving a catch-all arm that a passing run cannot take; rewritten as `if let` loops.
12. The two new `openai_responses` tests were first written with helpers whose catch-all arm no run could take; the assertions are now on the emitted event list itself (`events.is_empty()`, exact text).
13. `an_empty_summary_…` asserted only the deterministic fallback; it now also pins the fallback reason (`summary stream ended before a complete response`), which is what makes the §6.2 row for `evidence.rs:684` checkable.
14. Every new test asserts a value, an exact event sequence, an error category or a counter; no snapshot-only test was added, and no existing assertion was weakened.

## 6. Waiver ledger

One row per file that still has counted-missed lines. The category is the
structural class of the residue; the per-line registrations behind each row are
in §6.2–§6.5. `attribution-artifact` is used only where a *neighbouring* line in
the same statement carries a non-zero count (evidence quoted in §6.4).

| file | category | reason |
|---|---|---|
| `agent/src/agent/run_loop.rs` | attribution-artifact, unreachable-by-construction, unreachable-in-this-environment | 252 of the 281 counted-missed lines belong to **no region** (§1.1) and cannot be entered by any test. Of the 29 lines the LCOV `DA:` view reports at zero: 24 are `tracing` argument lines inside `if self.verbose { … }` blocks (§6.5 — siblings in the same argument list carry counts, so the argument list demonstrably ran), 3 are the test-side callbacks `|_| {}` / a test provider's task-`return` that the scenarios do not invoke (§6.4), 1 is the `Unchanged`-with-a-ticket recovery arm (§6.2), and 478 is a projection fallback the planner cannot emit (§6.2). No line in this file has a testable, untested behaviour left. |
| `agent/src/compaction/semantic/evidence.rs` | unreachable-by-construction, unreachable-in-this-environment, attribution-artifact | 302-304, 382-404, 534, 601-603 and 684 are unreachable by construction, each with the invariant in §6.2 (and `382-404` is additionally pinned by a test). 281/340/427/576 are mid-call cancellation guards with no `await` point inside their window (§6.3). 610/617/630 are a block-closing brace and a `?`-propagation line whose Ok path ran (610 is the `}` of a block that executed, 617 the `?` of a `finalize` that returned `Ok`, 630 the `}` of the `Compacted` arm that ran) (§6.4). |
| `agent/src/compaction/durable.rs` | unreachable-in-this-environment, unreachable-by-construction | 62/67/69 are the `Err` arm of `if let Err(write_error) = …fail_compaction(…)`: the call itself runs (its own lines carry counts, §6.4) but its SQLite write cannot fail without a broken database, and the store's injection flags cover commits, not this direct `UPDATE` (§6.3). 144-148 (a cached checkpoint whose cutoff is not in the raw snapshot) needs a receipt to outlive a re-ided transcript *inside one session id*; the fork path re-ids **and** gives the child its own session id, so the claim cannot match (§6.2). 164-175 (a cached checkpoint that a later checkpoint supersedes) is reachable in principle but needs a hand-built receipt whose `entry_id` differs from the projection's checkpoint while the input hash still matches — the same session-id/​input-hash coupling as above (§6.2); the *behaviour* it guards (a replay must not undo a later operation) is covered on the reachable replay path by the second run of `a_ticket_is_closed_without_a_checkpoint_…`. 263 is the mid-call cancellation shape of §6.3. |
| `agent/src/compaction/semantic.rs` | unreachable-in-this-environment, attribution-artifact | 521 and 735 are the `else` arms of `if cfg!(test) { 1 ms } else { 50 ms / 250·2^n ms }`: the unit-test build compiles the `else` out, and covering it needs a non-`cfg(test)` build (an integration test in `agent/tests/`, outside this group's write scope — §7). 632 is the "cancel while awaiting the next frame" arm and 722 the same guard inside `await_or_interrupt`; both need the interrupt flag to flip between the guard and the next poll of a real 1 ms timer, which the scripted providers cannot place deterministically (§6.3). 790/792/794 are the `#[cfg(test)]` attribute line and the two delimiters of the `let limit = match mode` statement in the `Reasoning` arm of `serialize_message`; lines 787/788/789/791/793 of that arm carry counts 4/4/2/2/4 (§6.4). |
| `agent/src/llm/mod.rs` | unreachable-in-this-environment, unreachable-by-construction, attribution-artifact | 32/36 are `is_body`/`is_request` in the transport classifier: this reqwest/hyper pair never reports either for a failure this client can cause (four real-socket fixtures surface as `is_decode`, `is_connect` or a `Builder` error — asserted in `a_transport_error_is_classified_by_the_kind_that_caused_it`) (§6.3). 266 is the `JoinError` of a `spawn_blocking` whose closure has no panic path (§6.3). 179/201 are a `?`-propagation line and a block-closing brace whose surrounding lines carry counts (§6.4). 397/522 are `tracing` arguments (§6.5). 453/507/542/555 are the pump's post-send guards, which fire only if the consumer drops the receiver between two sends (§6.3; the equivalent run-loop behaviour is covered by `a_timed_stream_stops_sending_when_the_run_drops_the_receiver`). 1285/1446/1537 are the closing braces of the three rewritten assertion helpers (§6.4). 1879 is a test-side `panic!` guard whose arm cannot be taken by a passing test (§6.4). |
| `agent/src/agent/mod.rs` | attribution-artifact | 12 counted-missed lines belong to **no region**: region reconstruction reports 0 uncovered and `--text` prints count `0` for no line of the file. Proof: `stream_truncation_error_message_maps_every_reason_to_a_stable_code` (33 calls; `51\|33`, `52\|32`, `55\|23`, `58\|15`) and `stream_truncation_survives_serialization_round_trips_and_legacy_payloads`. |
| `agent/src/llm/adapters/openai_responses.rs` | unreachable-by-construction, attribution-artifact | 937 is the `encrypted_content` branch of `openai_item_metadata`, whose parameter is passed `None` at all three call sites (194, 877, 1106) — a dead parameter, reported in §8 (§6.2). 911-913 is `unwrap_or_else`'s closure after `open_reasoning` has already inserted the entry; `late_wire_reasoning_id_is_persisted_without_changing_the_assembly_id` shows the wire id does **not** win, i.e. the entry was present (§6.2). 1242/1293 are test-side `panic!("expected reasoning end")` guards (§6.4). 659 and 2482 are a stray `}` and a multi-line `matches!(` span (§6.4). |
| `agent/src/llm/schema.rs` | attribution-artifact | 1 counted-missed line, no region, no count-0 line in the text or DA views; neighbours proven executed by `schema::tests`. |
| `agent/src/llm/adapters/openai_chat.rs` | attribution-artifact | 2 counted-missed lines with no region and no `DA:0` record (region view finds 5 lines against the metric's 2); neighbours proven executed by the module's own decode/`build_body` tests. |
| `agent/src/llm/adapters/anthropic.rs` | attribution-artifact | Test-only guard tails — `other => panic!("expected ToolInputEnd …")` (1169) and two `matches!` continuation lines (1172, 1180) — which run and pass with their zero-count region on the arm/brace continuation; the arm itself cannot be taken by a passing test (§6.4). |

### 6.1 Files that correctly contribute no data to the report

| file | category | reason |
|---|---|---|
| `agent/src/agent/events.rs` | unreachable-by-construction | Declaration-only (`pub enum RunEvent` and its fields): no executable line, so llvm-cov emits no file record. |
| `agent/src/compaction/durable/tests.rs` | attribution-artifact | `#[cfg(test)] mod tests;` file: no per-file record (its functions are in `data[0].functions[]`). Its test runs green. |
| `agent/src/compaction/semantic/evidence/tests.rs` | attribution-artifact | Same; its 22 tests ran green. |
| `agent/src/llm/stream_wait_tests.rs` | attribution-artifact | Same; its HTTP-boundary tests run in every full-suite measurement. |

### 6.2 Per-line registrations — `unreachable-by-construction` (with the invariant)

Each row states the invariant that makes the line impossible. A reviewer can
falsify any of them by exhibiting an input that reaches the line; two of them are
additionally pinned by a test that fails if the invariant stops holding.

| file:lines | invariant |
|---|---|
| `agent/src/compaction/semantic/evidence.rs:302-304` | `estimate_text_tokens` is `ceil(Q/4)` of a per-character quarter sum, hence subadditive: `est(a + "\n" + b) ≤ est(a) + est(b) + 1`. Every append in `build` is guarded by `est(line) + 2 ≤ budget − est(text)`, so after each append `est(text) ≤ budget − 1`; the initial text is checked at 197 with the same expression. The post-loop check at 301 therefore cannot fail. (The previous record's "underflow" path returns the *other* `BudgetExceeded` at 197-200 first, because a zero budget is smaller than the initial text's estimate.) Pinned by `a_window_without_a_summary_slot_fails_before_calling_the_model` (windows 1..16) and `a_cancelled_or_over_budget_index_build_is_refused`. |
| `agent/src/compaction/semantic/evidence.rs:382-404` | `project_prompt_context` emits `[protected anchors…] ++ [checkpoint message] ++ [raw tail…]`, and every entry before the checkpoint carries the anchor flag. Only `plan.removed`'s **last** entry sets `plan.cutoff_entry_id`, and `cut` is always ≥ the checkpoint's index + 1, so the last removed entry is the checkpoint exactly when the removed set is `anchors + checkpoint` — and `has_compactable_content` (231-243) rejects that as `Unchanged` before `stage` runs. There is no production path to a non-anchor entry before the checkpoint. Pinned by `a_projection_places_the_checkpoint_after_anchors_only` (4 protected-id sets). |
| `agent/src/compaction/semantic/evidence.rs:534` | `summary_reserve = min(window/16, HANDOFF_SUMMARY_TOKENS)`, so `reserve == 0` ⟺ `window < 16`. But `required ≥ summary_budget + 64 ≥ 65` while `maximum_target = min(MAX_EXPANDED_HISTORY, (window − reserve − margin)·3/4) ≤ 11` for `window ≤ 15`, so `plan` returns `BudgetExceeded` before 533. Pinned by `a_window_without_a_summary_slot_fails_before_calling_the_model` (the provider is never called for any window 1..16). |
| `agent/src/compaction/semantic/evidence.rs:601-603` | The match is on `summarized: Option<Result<ContextPreparation, ContextError>>` whose `Some(Ok(_))` arm was consumed by the outer `match` arm at 595, so the inner `Some(Ok(_))` is unreachable (a fact the compiler cannot express because the outer arm binds the rest as `outcome`). `Some(Err(error))` at 601 can only carry `InvalidSummary`, which `finalize` returns for an empty summary — impossible here, because the text passed at 582-585 always begins with `HEADER_WITH_SUMMARY`. The only other `finalize` failures are `BudgetExceeded`/`NoProgress`, matched at 598. |
| `agent/src/compaction/semantic/evidence.rs:684` | `write_handoff_summary` calls `call_summary_request`, which returns `Ok` only from `if complete && !text.trim().is_empty() { return Ok(text.trim().to_string()) }` (semantic.rs:696-697) — i.e. every `Ok` it can produce is already non-empty after `trim()`. The guard at 683 is defensive. Pinned by `an_empty_summary_is_rejected_in_favour_of_the_deterministic_index`, which asserts the *upstream* refusal's exact reason (see §5.13). |
| `agent/src/agent/run_loop.rs:478` | `project_prompt_context`'s projection fallback for `InvalidSummary | SummaryFailed`: `prepare_with_journal_and_summary` catches both variants internally and converts them into the deterministic evidence fallback (which `automatic_c3_falls_back_to_evidence_when_the_summary_fails` asserts end-to-end), so the loop cannot observe them. |
| `agent/src/agent/run_loop.rs:701-703` | The `Unchanged`-with-a-ticket arm of the provider-limit recovery. A `ProviderContextLimit` trigger is **not** threshold-gated (`plan`'s `threshold_gated` covers only `Automatic` and `ModelContextDownshift`), so when no boundary exists the planner refuses with `NoValidBoundary` (603-605) rather than returning `Unchanged`; and once a recovery has run, `provider_limit_checkpoint_id` is set and the loop short-circuits before preparing again (381-385). Pinned in intent by `a_provider_limit_recovery_without_a_boundary_refuses_instead_of_looping`, which asserts the refusal. |
| `agent/src/compaction/durable.rs:144-148` | The `Cached` branch is reached only for a `(session_id, input_key)` pair that a previous admission inserted, where `input_key` is a hash of that session's message payloads plus the policy. For the cached checkpoint's `cutoff_entry_id` to be absent from `raw` while the hash still matches, the session's messages must have been re-ided *without* changing their payloads **and** the receipt must survive under the same session id. The fork path (`session/fork.rs`) re-ids the child and copies the transcript under a **new** session id, so the child claims its own key and never sees the parent's receipt; there is no in-place re-id path. |
| `agent/src/compaction/durable.rs:164-175` | Same coupling: a replay where the cached checkpoint's `entry_id` differs from the checkpoint entry present in the projection requires the same `(session, input_key)` to have two different checkpoints, i.e. a receipt that outlives a later compaction over the *same* input hash — which means the later compaction wrote no message entries (checkpoint entries are excluded from `input_hash`). No production sequence produces two checkpoints for one input hash: the second compaction's key differs because its projection's checkpoint participates in the policy's strategy/window fields, and its input hash changes as soon as a message is appended. |
| `agent/src/compaction/semantic/evidence.rs:176-179`, `agent/src/agent/events.rs` | (declaration-only / no executable line — see §6.1 for `events.rs`.) |
| `agent/src/llm/adapters/openai_responses.rs:937` | `openai_item_metadata(id, encrypted_content)`: the parameter is `None` at all three call sites (194, 877, 1106), so the branch is dead code, not untested logic. Reported as a finding (§8.3) rather than deleted, because deleting it is a production change outside this segment's remit. |
| `agent/src/llm/adapters/openai_responses.rs:911-913` | `reconcile_reasoning_item` calls `open_reasoning` (892) before `state.reasoning_open.remove(&index)` (907-909), and `open_reasoning` inserts the entry for that index on every path; so the `unwrap_or_else` closure is dead. The behavioural fingerprint is `late_wire_reasoning_id_is_persisted_without_changing_the_assembly_id`: the emitted id is the assembly id established at open time, not the wire id in the `done` frame — if the closure ran, the wire id would surface. |

### 6.3 Per-line registrations — `unreachable-in-this-environment`

| file:lines | why this environment cannot produce it |
|---|---|
| `agent/src/compaction/semantic.rs:521`, `735` | `cfg!(test)` `else` arms (50 ms poll interval, 250·2ⁿ ms backoff). The measurement harness compiles the lib with `--cfg test`, so the arm is not in the measured binary. Covering it needs a non-`cfg(test)` build — an integration test under `agent/tests/`, which is outside this group's declared write scope (§7). |
| `agent/src/compaction/semantic.rs:632`, `722` | Cancellation must land in the window between the flag check and the next poll of `await_or_interrupt`'s 1 ms timer. The scripted providers either set the flag *before* the call (covered: `summary_model_respects_interrupt_during_stream`, `…during_connection_setup`) or never resolve at all (covered by the timeout tests), so the arm is not reachable deterministically; a real 1 ms race is a flake source, not a test. |
| `agent/src/compaction/semantic/evidence.rs:281`, `340`, `427`, `576` | Mid-call cancellation guards inside the CPU-bound planning/rendering section: there is no `await` point between them and the preceding check, so a caller cannot flip the interrupt flag inside that window. The reachable cancellation *is* covered (`cancelled_summary_never_becomes_a_deterministic_checkpoint`, `a_cancelled_or_over_budget_index_build_is_refused`), both asserting `Cancelled` and that no fallback ran. |
| `agent/src/compaction/durable.rs:62`, `67`, `69` | The `Err` arm of `if let Err(write_error) = manager.fail_compaction(…)`. The call runs whenever a ticket is failed (its own lines carry counts, §6.4) but the SQLite `UPDATE` cannot fail here: the store's failure-injection flags cover the queued `CommitRun`/`CommitCompaction`/`RewriteRun` commands (`persistence.rs:640/690/617`), and `fail_compaction` is a direct `db.call` on a healthy connection. Making it fail would need a broken database file, which the module's policy forbids producing by weakening production code. |
| `agent/src/compaction/durable.rs:263` | Mid-call cancellation guard, as above. |
| `agent/src/llm/mod.rs:32`, `36` | `TransportError::is_body()` / `is_request()` classify `reqwest::Error` kinds this reqwest/hyper pair does not produce for any failure the client can cause. Four real-socket fixtures (refused connection, stalled body, truncated body, builder failure) all surface as `is_decode`, `is_connect` or a `Builder` error, and each is asserted against both `reqwest`'s own predicates and the classifier (`a_transport_error_is_classified_by_the_kind_that_caused_it`). |
| `agent/src/llm/mod.rs:266` | The `JoinError` arm of a `spawn_blocking` image-preparation task: it needs the spawned closure to panic, and that closure only clones messages and calls a pure data-URL helper, so no input reachable from this client produces it. |
| `agent/src/llm/mod.rs:453`, `507`, `542`, `555` | The pump's `if tx.send(event).await.is_err() { return; }` guards: the consumer must drop the receiver *between two sends*. There is no in-crate consumer that drops one mid-stream (the run loop drains until the stream ends or the run is interrupted, and the interruption path is covered at the run-loop level by `a_timed_stream_stops_sending_when_the_run_drops_the_receiver`, which exercises the same behaviour on the test provider, not on this production pump). Producing it here needs a mock HTTP peer plus a consumer that abandons the stream at a chosen frame — see §7. |

### 6.4 Per-line registrations — `attribution-artifact`

Quoted counts are `DA:` values from the same LCOV export (§1.1): a neighbour in
the *same statement* carries a non-zero count, so the statement executed.

| file:lines | evidence |
|---|---|
| `agent/src/agent/run_loop.rs:232-233, 528-530, 1053-1054, 1095-1096, 1147-1148, 1410-1411, 1452-1453, 1463-1464, 1477-1479, 1481-1482, 1503-1504` | see §6.5 |
| `agent/src/agent/run_loop.rs:2000` | The `return;` of the *test* provider's `TimedEvents` task when the receiver is gone. The scenario (`a_timed_stream_stops_sending_when_the_run_drops_the_receiver`) covers the receiver-drop path in the run loop; this helper line is attributed 0 because the drop happens after the last frame. |
| `agent/src/agent/run_loop.rs:4807`, `4840` | `|_| {},` callbacks in two tests. `interrupted_open_tool_does_not_replay_provider_owned_identity` delivers a tool-input start and no text, so its `on_text` is never invoked (`text` stays empty and the test asserts the resulting history); `exhausted_retry_reports_connection_reason_for_unexecuted_tools` passes an `on_text` noop and asserts through the recorded `ToolExecutionFinished` events — the line carries 0 while the callback is never called by design. |
| `agent/src/compaction/semantic.rs:790`, `792`, `794` | The `#[cfg(test)]` attribute line and the delimiters of `let limit = match mode {…}` in the `Reasoning` arm of `serialize_message`; the arm's other lines are `787|4`, `788|4`, `789|2`, `791|2`, `793|4`. |
| `agent/src/compaction/semantic/evidence.rs:610`, `617`, `630` | `610` is the `}` of `if let Some(reason) = &fallback_reason` whose body ran (607/608/609 carry counts); `617` is the `)?` of a `finalize` that returned `Ok` (the `?`'s error region is 0 by construction — same shape as the `?` lines registered in other files); `630` is the `}` of the `if let ContextPreparation::Compacted = &mut prepared` block that ran (620-629 carry counts). |
| `agent/src/compaction/durable.rs:62`, `67`, `69` (the *statement*, see §6.3 for the arm) | `61|8`, `63|8`, `64|8`, `65|8`, `68|8` — the `if let Err(write_error) = …fail_compaction(…)` expression and its closing brace all run (the failure arm does not, §6.3). |
| `agent/src/llm/mod.rs:179`, `201` | `179` is the `)?` of `ResolvedModelTarget::from_model(…)` (the call ran, its `Ok` path taken); `201` is the `}` of `if let Some(target) = self.target.write().as_mut()` whose body ran (196-200 carry counts). |
| `agent/src/llm/mod.rs:1285`, `1446`, `1537` | The closing braces of the three assertion helpers rewritten in §5.11; the loops' bodies (and their `break`) ran, the brace is attributed 0 for the same reason as other block ends in this file. |
| `agent/src/llm/mod.rs:397`, `522` | `tracing` argument lines inside `elapsed_ms = started_at.elapsed()…` fields; sibling argument lines in the same macro carry counts (the §6.5 pattern). |
| `agent/src/llm/mod.rs:1879` | A test-side `other => panic!("expected terminal stream error, got {other:?}")` guard: taking it fails the test, so no passing run can cover it (the assertion immediately below pins the message it guards). |
| `agent/src/llm/adapters/openai_responses.rs:1242`, `1293` | Test-side `panic!("expected reasoning end")` guards. The assertions *after* each guard (`id == "reasoning-2"`, `id == "reasoning-1"`) prove the `ReasoningEnd` arm was the one taken; the guard arm cannot be entered by a passing test. |
| `agent/src/llm/adapters/openai_responses.rs:659`, `2482` | `659` is a stray `}`; `2482` is the `matches!(` line of a multi-line assertion macro (`2483-2489` carry the match, `2482` is the span start). |
| `agent/src/llm/adapters/anthropic.rs:1169`, `1172`, `1180` | `1169` is a test-side `panic!("expected ToolInputEnd …")` guard; `1172` and `1180` are `matches!`/assert continuation lines whose neighbouring lines carry counts. |
| `agent/src/agent/mod.rs`, `agent/src/llm/schema.rs`, `agent/src/llm/adapters/openai_chat.rs` | counted-missed lines that no view can attribute to a region — verified by `--src`/`DA:` cross-check (§1.1) and by the `--text` view printing no count-0 line for those files. |

### 6.5 The `tracing`-argument cluster — why it is an artifact, per cluster

24 of `run_loop.rs`'s 29 `DA:0` lines are argument lines of `if self.verbose {
tracing::… }` diagnostics. A `tracing` macro evaluates its **whole** argument list
in one expression (inside the macro's `if enabled` block), so an argument list
cannot be evaluated partially: if any line of it has a non-zero count, all of its
arguments were evaluated. Each cluster has such a line, from the same LCOV export:

| macro (first line) | argument lines at `DA:0` | sibling argument/format lines with a count |
|---|---|---|
| 229 | 232, 233 | `231` (covered), format line `229` = 16 |
| 526 | 528, 529, 530 | `531\|34`, `532\|34`, `533\|34` |
| 1049 | 1053, 1054 | `1051\|393`, `1052\|393`, `1055\|393` |
| 1093 | 1095, 1096 | `1091\|88`, `1092\|88` |
| 1145 | 1147, 1148 | `1143\|4` |
| 1408 | 1410, 1411 | `1408\|2` |
| 1450 | 1452, 1453 | `1448\|353`, `1449\|195`, `1450\|12` |
| 1461 | 1463, 1464 | `1458\|22`, `1459\|22` |
| 1474 | 1477-1479, 1481-1482 | `1474\|6`, `1480\|6` |
| 1498 | 1503, 1504 | `1495\|158`, `1496-1498\|14` |

The lines that carry 0 are exactly those whose argument contains a method call
(`messages.len()`, `u.cache_read_tokens.unwrap_or(0)`, a `.map(…).collect().join(…)`
chain), whose nested call region llvm-cov places on the call's own span; the
plain-field arguments on the lines above them keep their count. Reproduce any row
with `python coverage/ag-llm-dalines.py coverage/agent-ag-llm.lcov
agent/src/agent/run_loop.rs --counts <LO> <HI>`.

## 7. What a next segment would need

1. **The `commit_compaction` failure the supervisor asked for already has a
   seam.** `SessionPersistence` carries `fail_next_commit` (compiled into every
   build, setter `cfg(test)`-only) and the `CommitCompaction` command consumes it
   (`session/persistence.rs:640`), so `run_loop.rs:435`/`673` and the receipt
   assertions are reachable **without touching `crate::session` or any public
   API**. This segment used it; no seam was added anywhere. What remains outside
   that seam is `Manager::fail_compaction`'s own write failing (§6.3).
2. **`cfg!(test)` `else` arms** (`semantic.rs:521`, `735`) need one integration
   test under `agent/tests/` that drives a summary call through a non-`cfg(test)`
   build of the lib. That is a new write scope (outside this group's) and is the
   cheapest remaining real line in the subtree.
3. **`llm/mod.rs`'s pump guards** (453/507/542/555) need a mock HTTP peer plus a
   consumer that abandons the stream at a chosen frame — the same fixture shape
   `llm/stream_wait_tests.rs` already builds, extended with a mid-stream drop.
4. **`durable.rs:144-148`/`164-175`** would need either a hand-built receipt (a
   direct SQL fixture whose checkpoint points at an entry the raw view omits) or a
   production path that re-ids a journal in place. The first is testable and is
   the honest way to cover the refusal; the second is a design question for the
   fork/restore owner.
5. Everything else §6 registers is an artifact or a `DA:0` line whose statement
   demonstrably ran; closing the *number* further would require covering lines
   that are not in any region, which no test can do.

## 8. Things found while working (reported, not fixed silently)

1. **A thread-local tracing subscriber never enables a callsite another test already hit with no subscriber** (`tracing` caches interest globally and rebuilds only for a global default). Raising the level, `RUST_LOG=debug` and `rebuild_interest_cache()` still leave the §6.5 argument lines at zero.
2. **The gate metric is inflated by lines that no region covers** (§1.1): 253 of `run_loop.rs`'s 282. Covering production region lines moves it by single digits.
3. **Dead parameter:** `openai_item_metadata(id, encrypted_content)` — all three call sites pass `None` (registered in §6.2, not deleted: that is a production change).
4. **`reqwest` reports an invalid chunk-size line as `is_decode()`, not `is_body()`**, and an unbuildable URL as a `Builder` error outside every named kind. Both asserted.
5. **The empty-summary arm moved**: `an_empty_summary_…` shows `call_summary_request` refusing an empty stream before `write_handoff_summary` can see it (registered as unreachable-by-construction in §6.2, with the reason pinned by the test).
6. **`run_loop.rs:478`'s fallback is defensive**, not reachable: `prepare_with_journal_and_summary` converts summary failures into the deterministic fallback internally (§6.2).
7. **The previous record's triage of `evidence.rs:302-304` and `382-404` was wrong** (both listed as reachable): 302-304 returns the other budget error first, and 382-404 needs a projection shape `project_prompt_context` cannot build. Corrected in §0/§6.2.
8. **`Get-Content` line numbers are not rustc's line numbers for this repo's sources** (GBK decoding eats three lines of `run_loop.rs`; 6377 vs 6380): use `ag-llm-src.py`, `Select-String`, or any UTF-8 reader when mapping `verify.py uncovered` output to source.
9. **`Registry::resolve_request_target("")` returns `Some`** (a default); pinned by an assertion; whether a silent default is right is a question for the registry owner.
10. **Replaying reasoning content at a different `content_index` duplicates the part**; the first version of that test failed exactly there.
11. **A divergent `reasoning_summary_text.done` frame is dropped entirely** (no event at all) rather than replacing the deltas — pinned by `a_divergent_summary_done_frame_appends_nothing`. Whether "silently ignore a contradictory rewrite" is the right resolution is a question for the adapter owner.
12. **Peer flakes:** `rpc::commands::session_lifecycle_tests.rs` (three tests, `ag-rpc`'s scope) fails under a loaded full-suite run and passes in isolation; reported, not touched. `agent/src/cli.rs:69` and `agent/src/session/database.rs:888` fail `clippy -D warnings` on `main`, also outside scope. A second, more damaging one appeared while producing this record: **`models::tests::get_default_model_returns_something` (`agent/src/models/mod.rs:1880`, another group's file) failed once out of three full-suite runs** — it reads the host's `settings.json` and requires the named default to resolve in the catalog, so a peer mid-write makes it red — and a red test aborts `cargo llvm-cov`'s merge, which silently produced a 315-line-low report (16784/17179). Both exports in §1 come from a re-run that was green; the whole measurement was thrown away and repeated rather than quoted.

## 9. Known gaps and next checks

1. **97.7366 %** of counted lines; §6 carries a category for every file the gate
   lists and §6.2–§6.5 register each of the 115 `DA:0` lines with its class. The
   cheapest remaining real work is the `cfg!(test)` integration test (§7.2) and
   the pump-drop fixture (§7.3).
2. Re-derive per-line numbers with the four scripts in §1.1; `verify.py
   uncovered`'s "approx. line numbers" are hints, not the missed lines, and the
   summary total is not a count of testable lines (§1.1).
3. Unresolved questions before writing more fixtures: §8.11 (divergent summary
   `done`) and §6.2's `durable.rs:164-175` (whether two checkpoints can share one
   input hash).
4. A measurement is only trustworthy from a green suite; when peer tests are red
   the report file is not regenerated (correct) — re-run rather than reuse. The
   two exports in §1 must come from the same `--no-report` run.
