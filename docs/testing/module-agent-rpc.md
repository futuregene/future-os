# Module `agent/rpc` + `agent/grpc` (worker group `ag-rpc`) — coverage handoff

Subtree scope (frozen by `verify.py` `RUST_GROUPS["ag-rpc"]`): `agent/src/rpc/` and `agent/src/grpc/`.
Gate: `python .future/cov100/verify.py rust-module ag-rpc 99.99 docs/testing/module-agent-rpc.md coverage/agent-ag-rpc-report.json`.
Evidence token for this run: **`gate-green`** (see §8 for the exit status of that exact command).

## 1. Run identity and the exact measurement command

* Task `goal_6d61125ba837` / `todo_a6074c81c7c9` (this revision), subtree **ag-rpc**, crate `future-agent`.
* Private build dir `target/cov-w-ag-rpc3` via `CARGO_TARGET_DIR` (`cargo-llvm-cov 0.9.1` has no `--target-dir`).
  Named after the agent id, not the crate: the previous revision used `target/cov-ag-rpc2`, and a crate-named dir
  is how two workers collided earlier in this goal.
* Report consumed by the gate: `coverage/agent-ag-rpc-report.json` (gitignored, not committed).
* Upstream revision when this was measured: `c173e64e` plus this branch's edits; the measurement ran green
  (**2012 passed / 0 failed / 1 ignored**).

```powershell
$env:CARGO_TARGET_DIR = "target/cov-w-ag-rpc3"
# THE measurement: atomic (test run + report in one invocation), every test included,
# no exclusion list and no name filter. --test-threads=4 is a run setting, not a filter:
# all 2012 lib tests of the crate run (1 ignored, pre-existing). Wall time ~15 min.
cargo llvm-cov -p future-agent --lib -j 3 --json `
    --output-path coverage/agent-ag-rpc-report.json -- --test-threads=4

python .future/cov100/verify.py rust-module ag-rpc 99.99 `
    docs/testing/module-agent-rpc.md coverage/agent-ag-rpc-report.json
```

The per-line view quoted in §6 comes from a **separate, non-measuring** export of the same profile data:
`cargo llvm-cov report --lcov --output-path coverage/ag-rpc6.lcov` (same `CARGO_TARGET_DIR`). It is used only to
locate code — the gate's per-file `uncovered` counts are the authoritative numbers (§2), and the two views
disagree by design (F11).

### Why this exact invocation

1. **Atomic only.** `cargo llvm-cov report` as a separate step is unsafe in this checkout: after a run whose
   test target fails, the raw profiles are consumed and the standalone `report` re-merges stale/foreign
   `.profraw`, producing an all-zero report (`0/60903` was observed and caught by the gate's own
   "broken measurement" check). One command, or no number. (The non-measuring `--lcov` export above is a
   *read* of the profile data the measuring run already produced in this same private target dir; it plays no
   part in §2 or §8.)
2. **No exclusion list.** An earlier attempt produced a report with three pre-existing load-sensitive tests
   skipped; that report read **96.59%** — the skipped tests account for ~123 covered lines in this subtree,
   so a skip list materially *understates* it. The final measurement runs every test.
3. **`--test-threads=4`.** The whole crate's tests are instrumented and run in one process; at full
   parallelism several pre-existing liveness deadlines expired (see §7 F6–F8). Lower parallelism keeps every
   test running while making the deadline-based tests reliable here. Coverage is unaffected by the setting.
4. **No name filter.** The previous revision measured `-- rpc:: grpc::` because the shared checkout did not
   compile (F2). This revision's whole-`--lib` run was green (2012 passed / 0 failed / 1 pre-existing
   ignored), so the unfiltered report the task asked for is the one consumed here.
5. **`-j 3`.** Several workers share this box; the dispatched budget is small on purpose (plan §Concurrency).

## 2. Before / after (real llvm-cov line coverage)

| measurement | lines | % | uncovered | files |
|---|---|---|---|---|
| before (dispatch baseline, `coverage/agent-uncovered-final.txt`) | 13777/14294 | 96.3831% | 517 | 16 |
| previous revision of this doc (§2 of the filtered run, `-- rpc:: grpc::`) | 14263/14598 | 97.7052% | 335 | 16 |
| previous revision, final report, all tests (`target/cov-ag-rpc2`) | 15026/15302 | 98.1963% | 276 | 16 |
| this revision, intermediate pass (same tree minus a token-only clippy fix) | 15094/15336 | 98.4220% | 242 | 16 |
| **after — this revision, final tree, all tests, `coverage/agent-ag-rpc-report.json`** | **15092/15338** | **98.3961%** | **246** | **16** |

**−30 uncovered lines this revision** (276 → 246), concentrated exactly where the task pointed — and both passes
agree on that: the intermediate pass read 242 for the same reason the final one reads 246, i.e. the known
±3-line per-file movement of `settings.rs`/`session.rs` under load (F10). Every other file's count is identical
between the two passes and to the previous revision's table.

| file | before | after | delta |
|---|---|---|---|
| `agent/src/rpc/session_prompt.rs` | 27 | **5** | **−22** |
| `agent/src/rpc/commands/session_lifecycle.rs` | 12 | **4** | **−8** |
| `agent/src/rpc/session.rs` | 20 | 20 | ±0 (read 19 in the intermediate pass — F10; a token-only clippy fix in one of its tests landed between the two passes) |
| `agent/src/rpc/commands/settings.rs` | 55 | 55 | ±0 (read 52 in the intermediate pass — F10; the file is untouched here) |
| `agent/src/rpc/mod.rs` | 34 | **34** | **±0 — see F15**: the lines this file's row listed as the
  activation-failure arms were *branch/region* artifacts and were already line-covered; the real gap was in the
  two callers above. The 9 instrumented lines the new seam adds are all covered, so the file's total grows
  1216 → 1225 and its uncovered count stays flat. |
| (other 11 files of the subtree) | — | — | unchanged |

The denominator grows because the new tests and the two `#[cfg(test)]` seams are instrumented lines inside the
same subtree; the honest figure is the uncovered count.

Per-file remainder, subtree only (`coverage/ag-rpc-json-gaps.py` re-derives it, sorted by uncovered):

| uncovered | covered/total | file |
|---|---|---|
| 55 | 532/587 | `agent/src/rpc/commands/settings.rs` |
| 54 | 1962/2016 | `agent/src/rpc/approval.rs` |
| 34 | 1191/1225 | `agent/src/rpc/mod.rs` |
| 24 | 966/990 | `agent/src/grpc/mod.rs` |
| 22 | 1857/1879 | `agent/src/rpc/protocol.rs` |
| 20 | 4001/4021 | `agent/src/rpc/session.rs` |
| 11 | 436/447 | `agent/src/rpc/commands/providers.rs` |
| 5 | 1269/1274 | `agent/src/rpc/session_prompt.rs` |
| 5 | 350/355 | `agent/src/rpc/run_snapshot.rs` |
| 4 | 521/525 | `agent/src/rpc/commands/session_title.rs` |
| 4 | 495/499 | `agent/src/rpc/commands/session_lifecycle.rs` |
| 3 | 21/24 | `agent/src/rpc/commands/skills.rs` |
| 2 | 679/681 | `agent/src/rpc/prompt_helpers.rs` |
| 1 | 227/228 | `agent/src/rpc/commands/mod.rs` |
| 1 | 237/238 | `agent/src/rpc/commands/observability.rs` |
| 1 | 246/247 | `agent/src/rpc/commands/run_control.rs` |
| 0 | 102/102 | `agent/src/rpc/commands/test_support.rs` |

## 3. Files changed — test code only

**No production line was touched**: no guard removed, no `#[cfg(test)]` wrapped around production code, no
type widened, no assertion weakened. Every change is inside a `#[cfg(test)] mod tests` block (or a test-only
helper), plus one new test module registration.

**This revision (`todo_a6074c81c7c9`)** — the two places the task named, plus one clippy fix inside the same subtree:

| file | change |
|---|---|
| `agent/src/rpc/mod.rs` | +2 `#[cfg(test)]` items and one `#[cfg(test)]` block inside `activate_persisted_session` (the activation-failure hook, §3a.3); +4 tests live in `commands/session_lifecycle_tests.rs` |
| `agent/src/rpc/commands/session_lifecycle_tests.rs` | +4 tests: the post-commit activation failure through `fork` and through `clone`, the failure injected directly on `activate_persisted_session`, and the cold (first) activation announcement |
| `agent/src/rpc/session_prompt.rs` | +1 `#[cfg(test)]` registry and its capture/`struct` items, plus the `#[cfg(test)]` capture call in `prompt_internal` (§3a.4) |
| `agent/src/rpc/session_prompt/tests.rs` | +3 tests invoking the captured production callbacks (checkpoint commit, escalation, `tool_sandboxed`), + `run_fixture_with_id` / `take_run_callbacks` / `test_checkpoint` helpers (the pre-existing `run_fixture` is now a one-line delegate) |
| `agent/src/rpc/session.rs` | clippy fix in the previous revision's test `manual_compaction_with_nothing_left_to_compact_fails_its_receipt`: `vec![…]` → `[…]` (`clippy::useless_vec`), which `-D warnings` would have failed on. Semantics unchanged (the binding is only read through `iter`/`iter_mut`/indexing), line count unchanged, and the test was re-run green. No coverage-bearing change. |

Previous revision's files (unchanged by this revision) follow.

| file | change |
|---|---|
| `agent/src/rpc/commands/skills_tests.rs` | **new**: 5 tests driving the skill handlers under a redirected `$HOME` |
| `agent/src/rpc/commands/mod.rs` | registers `#[cfg(test)] mod skills_tests;` |
| `agent/src/rpc/commands/dispatcher_tests.rs` | +3 tests (tool-inspection scope, storage failures, unknown session id) |
| `agent/src/rpc/commands/observability_tests.rs` | +2 tests (`get_messages` projection, cross-platform `export_html`) |
| `agent/src/rpc/commands/session_lifecycle_tests.rs` | +4 tests (skipped-migration paging, cache-hit reads, client provenance on clone, self-dispatch of the tool/output handlers was **not** needed) |
| `agent/src/rpc/commands/settings_tests.rs` | +1 test (shell timeout clamp) |
| `agent/src/rpc/commands/session_title.rs` | +2 tests (`suggest` guard arms; real provider selection through the handler) |
| `agent/src/rpc/session.rs` | +9 tests, 12 weak tests repaired, one liveness deadline widened (§7 F6); previous revision: +8 tests, 2 weak tests repaired (one of them renamed, F9) |
| `agent/src/rpc/protocol.rs` | +2 tests driving the event-batch writer directly and a failing batch commit (journal health, dead worker, blocked read path); previous revision: +3 tests and a `#[cfg(test)]`-only worker gate + writer hooks (§3a), and 1 weak test repaired (sleep → join) |
| `agent/src/rpc/session_prompt/tests.rs` | one liveness deadline widened (§7 F7) |
| `agent/src/rpc/mod.rs` | +2 tests (legacy broadcaster alias, `publish_session_created` wrapper) |
| `agent/src/rpc/run_snapshot.rs` | +1 table test (`carries_arguments`) |

### 3a. The test-only seams (previous revision's two, then this revision's two)

All are `#[cfg(test)]`-only and change **no** production behaviour, default path, error message or public
API: in a non-test build the added items do not exist, and every injection point is unreachable because its
only setter/hook is `#[cfg(test)]`.

1. **`agent/src/rpc/protocol.rs` — event-persistence-worker handle.** The writer already owned its worker
   thread and command channel; the seam exposes them to tests instead of making a test race wall-clock:
   * `EventWriteCommand::Hold(Barrier)` (`#[cfg(test)]` variant, `#[cfg(test)]` match arm) parks the worker
     inside its command loop, so a test can act on the channel at a known point in the worker's lifecycle.
   * `EventBatchWriter::hold_worker_for_test()` — sends the gate and returns it.
   * `EventBatchWriter::disconnect_worker_for_test()` — drops the only command sender while the worker is
     parked, and installs a replacement whose receiver is already gone, so the writer's own `Drop` handshake
     is skipped instead of blocking on a worker that has exited.
   * `EventBatchWriter::stop_worker_for_test()` — retires the worker through its real `Shutdown` handshake and
     **joins** the thread, replacing the previous test's `sleep(200ms)` with an observed fact.
2. **No new seam was needed for `session.rs`.** `SessionPersistence` already has test-only failure injection
   (`fail_next_commit`, `kill_worker_for_test`, `reset_error` from the pre-existing test-support pattern), so
   the compaction-ticket failures are injected through the existing `fail_next_commit` hook and the real
   journal/store. Nothing in `agent/src/session/` was modified (and `agent/src/session/` is outside this
   task's declared write set — see F12 for the one arm that would need a new seam there).
3. **`agent/src/rpc/mod.rs` — post-commit activation failure (this revision).** Same shape as the two hooks
   already in this file (`GET_SESSION_PRE_INSERT_HOOK`, `RELOAD_RACE_HOOK`):
   * `ACTIVATE_PERSISTED_SESSION_FAIL_HOOK: parking_lot::Mutex<Option<String>>` (`#[cfg(test)]` static) and
     `fail_next_session_activation_for_test(&str)` (`#[cfg(test)]` private fn) arm the failure for exactly one
     session id.
   * The check sits at the **top** of `activate_persisted_session`, before `try_get_session`, so the injected
     failure has the same observable footprint as the real hydrate failure it stands in for: nothing is
     hydrated, registered, or announced, and the durable journal is untouched. The hook is consumed on use, so
     the next activation of the same id succeeds. In a non-test build the block does not exist and the
     function body is byte-identical to before.
4. **`agent/src/rpc/session_prompt.rs` — run callbacks (this revision).** The three callbacks the run task
   receives are the run's own `Arc`s; the seam only keeps a test-only handle on them:
   * `RunCallbacksForTest { on_checkpoint, escalation, on_sandboxed }` + `RUN_CALLBACKS_FOR_TEST`
     (`LazyLock<Mutex<HashMap<String, RunCallbacksForTest>>>`, keyed by session id, mirroring the file's
     existing `SCHEDULED_DEQUEUE_HOOK`) + `capture_run_callbacks_for_test(...)`.
   * One `#[cfg(test)] { capture_run_callbacks_for_test(&session_id, checkpoint_callback.clone(),
     escalation.clone(), on_sandboxed.clone()); }` block after the closures are built (the two `erased`
     clones are themselves `#[cfg(test)]` bindings). Nothing is captured in a non-test build; the clones
     therefore cost production nothing.
   * Why a capture instead of a real run: `escalation`/`on_sandboxed` are only invoked by the tools layer on a
     host with a working sandbox backend (this host has none, which is why the pre-existing
     `run_sandbox_denial_escalates_through_session_wiring` returns early), and `on_checkpoint` is **shadowed by
     the compaction ticket** in this wiring (see F16) — a compaction journal and the checkpoint callback are
     installed under the same `!is_ephemeral` condition, and whenever a journal is present
     `prepare_with_journal_and_summary` returns `Some(ticket)`, so `run_loop`'s `ticket.finish(...)` branch
     always wins and `ctx.on_checkpoint` is unreachable *at runtime*. The tests invoke the captured closures
     with production inputs and assert the effects (durable checkpoint entry, published approval exchange,
     published `tool_sandboxed` event).

### New tests this revision (7)

| test | file | what it pins down |
|---|---|---|
| `activate_persisted_session_reports_a_post_commit_failure_and_keeps_the_session_readable` | `commands/session_lifecycle_tests.rs` | the committed session's activation fails: the error is **returned** (not swallowed), the live registry gains nothing, the durable entry count **and** journal revision are unchanged, and a later activation loads the full committed transcript |
| `fork_reports_a_post_commit_activation_failure_and_keeps_the_committed_child` | same | through the RPC surface: `success:false` with `fork committed but could not be activated: …`, the child stays on disk with its history, the retry replays the same commit (`created:false`, same id, no duplicate entries) |
| `clone_reports_a_post_commit_activation_failure_and_keeps_the_committed_child` | same | the same contract for `clone`, incl. the parent's in-memory-message guard and the clone's own provenance path |
| `activating_a_cold_persisted_session_announces_the_first_activation` | same | a committed-but-not-resident session activates, is registered, and is announced on the global stream with its own id; the second activation returns the **same** `Arc` and announces nothing |
| `checkpoint_callback_commits_the_checkpoint_durably_and_reports_a_failed_commit` | `session_prompt/tests.rs` | the run's own checkpoint callback: its entry reaches the journal with `EntryType::Compaction` and the checkpoint identity in the body; a closed persistence boundary makes the callback **return** the error, and the failed commit leaves exactly one checkpoint entry behind |
| `escalation_callback_publishes_a_decidable_sandbox_request_on_the_session_stream` | same | the run's own escalation requester: the request is published to the session's subscribers (`approval_request`, `kind:"sandbox_escalation"`, the command, this session's id), the decision arrives from the session's gate and is returned as `Approved`, and `approval_decision:approved` is published for the same request id |
| `sandboxed_notifier_broadcasts_tool_sandboxed_for_the_run` | same | the run's own notifier: one `tool_sandboxed` event on the session's stream carrying `type` and the exact command |

### New tests (11) — previous revision

| test | file | what it pins down |
|---|---|---|
| `manual_compaction_receipt_failure_is_remembered_and_not_retried` | `session.rs` | one admitted compaction takes exactly one receipt; when the durable commit fails the ticket is marked **failed** and the identical operation is refused with `compaction_previous_failed` **without a second model call** |
| `manual_compaction_with_nothing_left_to_compact_fails_its_receipt` | same | a no-op manual compaction (projection is the previous checkpoint) still owns a receipt; its failed close-out is refused on retry and appends no second checkpoint entry |
| `ephemeral_session_checkpoint_commit_failure_is_reported` | same | the no-journal path commits through `commit_checkpoint`; a failure there releases the admission fence, broadcasts `compaction_failed` and never `compaction_committed` |
| `compact_checkpoint_commit_failure_broadcasts_and_errors` (rewritten) | same | the claim now really succeeds, so the error asserted is the injected commit failure — not "session does not exist" |
| `compact_provider_failure_degrades_to_the_evidence_index` (rewritten) | same | a failing summarizer degrades to `summaryOutcome.status="evidence_only"` with the provider error as the reason, still commits a durable checkpoint, charges nothing, and reports committed — see F9 |
| `manual_summary_without_a_reported_price_is_priced_from_the_model_rates` | same | a provider that reports tokens but no `credit_cost` is priced from the model's own rates: totals and the 4-way split are both the computed amounts |
| `a_journal_with_rows_it_cannot_read_still_prices_the_rows_it_can` | same | 5 unreadable/unknown journal shapes are skipped (no usage body, non-object `data`, absent `data`, unknown event type, unpriced model) while the readable request is still priced |
| `a_replayed_cost_split_still_applies_when_its_cache_write_fails` | same | a closed persistence queue makes the cache write fail; the split still applies in memory and nothing is cached (the next open replays rather than trusting a row never written) |
| `a_failed_setting_write_still_applies_in_memory_and_is_not_swallowed` | same | `set_auto_compaction` on a session whose `session_info` is unreadable: the live flag changes, the durable update fails without a half-written row |
| `event_batch_writer_refuses_durable_writes_after_a_failed_batch` | `protocol.rs` | once a batch commit failed, the worker refuses later durable appends even after the shared health flag is cleared, and a flush with a pending event reports the store error instead of answering `Ok` |
| `event_batch_worker_exits_when_its_last_sender_disappears` | same | both loss points: the worker parked in `recv()` with an empty batch, and in `recv_timeout()` with a pending one — the pending event is **not** committed late, the store cursor does not move, and the writer fails closed |

### New tests (28) — previous revision
| `list_installed_skills_reports_both_scopes_from_disk` | `commands/skills_tests.rs` | global/app scope, `external` source, frontmatter version, canonicalized `location`, a dir without `SKILL.md` is not a skill |
| `sync_skills_without_auto_upgrade_reports_a_noop_and_refreshes_discovery` | same | auto-upgrade off ⇒ no platform call, four empty result arrays, discovery republished |
| `uninstall_skill_removes_the_directory_and_is_idempotent` | same | the directory is deleted; a second delete is `removed:false`, not an error |
| `invalid_skill_identifiers_are_rejected_before_any_state_changes` | same | `""`, `..`, `a/b`, `a\b`, spaces, trailing `.`, >128 B rejected for install **and** uninstall, no residue |
| `platform_backed_skill_commands_fail_closed_when_the_platform_is_unreachable` | same | `base_url` at a closed port: catalogue/install report failure, no half-installed skill |
| `tool_inspection_requires_its_whole_scope_and_reports_an_empty_page` | `commands/dispatcher_tests.rs` | missing/empty `runId`/`toolCallId` rejected; empty run = empty page, `hasMore:false`, cursor unmoved |
| `storage_failures_are_reported_with_their_own_error_code` | same | unopenable `agent.db`: three entry points return `session_storage_unavailable`/retryable; unhydrated session says "Unable to restore session context" |
| `get_session_entries_rejects_an_empty_or_unknown_session_id` | same | no silent fallback to another session; store stays usable |
| `get_messages_lifts_the_run_id_out_of_the_metadata_blob` | `commands/observability_tests.rs` | `runId` lifted out of `metadata`, other keys kept, CJK body as a typed `text` block |
| `export_html_writes_the_transcript_to_the_configured_directory` | same | success path (was `cfg(not(windows))`) now exercised on Windows via the test-only dir override |
| `switch_session_reports_an_unreadable_store_as_retryable` | `commands/session_lifecycle_tests.rs` | damaged store ⇒ `session_storage_unavailable`, `retryable:true` |
| `delete_session_refuses_while_compaction_holds_the_session` | same | `session_busy`/`busy_reason:compaction`, session not `deleting`; accepted when compaction ends |
| `paged_history_reports_a_skipped_migration_as_unreadable` | same | a `legacy_imports.status='skipped'` marker makes paged reads fail with `session_history_unreadable`; unpaged reads still work |
| `repeated_entry_reads_are_served_from_the_cached_projection` | same | second read hits the revision-keyed display cache (asserted on the cache, not just on equal output) |
| `clone_uses_the_requesting_clients_provenance_when_it_is_supplied` | same | client `created_by`/`creator_id`/`client_request_id` win over the parent's and the RPC id |
| `shell_timeout_is_clamped_to_the_documented_bounds` | `commands/settings_tests.rs` | `1 ms`, `60 s`, `u64::MAX` and `0` all run the command (5 s…30 min clamp + policy default) |
| `suggestion_rejects_oversized_tool_calling_and_contentless_answers` | `commands/session_title.rs` | >8192 B ⇒ "too long"; quotes-only ⇒ "empty title"; `ToolInputStart` ⇒ "must not call tools"; reasoning-only ⇒ "without a complete response" |
| `a_real_title_request_selects_the_configured_provider_and_fails_closed` | same | credentialed provider at a closed port: model selection runs, the failure is the transport (not "Session model is unavailable") |
| `title_requests_reject_an_unsupported_locale_and_content_free_sessions` | same | locale outside `{zh,en}` rejected; content-free session ⇒ exact localized message in both locales |
| `set_thinking_level_maps_every_level_to_its_budget` | `session.rs` | all six levels + unknown ⇒ live loop budget 0/2000/4000/8000/16000/24000/0 |
| `reconcile_model_reference_keeps_an_available_identity_and_swaps_a_ghost` | same | available identity untouched; unavailable one replaced and the live loop follows |
| `reconcile_model_reference_without_any_provider_is_an_error_that_changes_nothing` | same | no provider ⇒ error, stored identity unchanged |
| `execute_shell_cancellation_wins_over_a_long_timeout` | same | stale cancel generation beats a 30 s timeout, promptly |
| `execute_shell_timeout_terminates_a_windows_child` | same | `#[cfg(windows)]`: a 300 s child is terminated on a 200 ms timeout |
| `publish_session_created_wrapper_announces_with_an_empty_creator` | `mod.rs` | legacy 3-arg wrapper emits `session_created` with empty `creatorId` |
| `global_config_broadcaster_is_the_global_events_broadcaster` | same | deprecated alias is `Arc::ptr_eq` with the global broadcaster |
| `tool_start_arguments_are_only_significant_when_present_and_non_empty` | `run_snapshot.rs` | 9-case table over the argument payload space |
| `event_batch_writer_refuses_appends_when_unhealthy_or_when_its_worker_is_gone` | `protocol.rs` | healthy durable append commits; an unhealthy journal refuses with its recorded error and writes nothing; a dead worker fails the enqueue and marks the journal unhealthy |
| `a_failed_batch_commit_is_reported_and_blocks_the_read_path` | same | `DROP TABLE run_events` ⇒ the commit failure is recorded as a persistence error and the read path errors instead of returning a truncated history |
| (plus the 12 repaired tests listed in §5) | `session.rs` | see §5 |

## 4. Dimensions — evidence per dimension

| dimension | evidence in this subtree |
|---|---|
| `boundary` | full thinking vocabulary incl. unknown value; missing/empty/`None` tool scope; empty session id; 9-shape argument-payload table; 8192-byte title overflow; >128-byte skill id; odd `shell_timeout_ms` (`1`, `u64::MAX`, `0`); CJK bodies (`怎么导出？`, `缓存我`) round-tripped through projection and HTML export; a skill dir without `SKILL.md` not counted |
| `error-path` | unopenable store ⇒ typed retryable failures from 3 entry points + hydration gate; damaged store for `switch_session`; compaction-busy refusal for `delete_session`; path-traversal skill ids with zero state change; platform unreachable ⇒ fail-closed install/catalogue; `suggest` arms (oversized, empty, tool call, no completion); unsupported title locale; title request reaching the provider then failing closed; nonzero shell exit (pre-existing); previous revision: the compaction receipt commit failing (ticket `finish` → `fail`, error propagated, receipt left failed, no retry, no state advanced); the same for a no-op ticket and for the ephemeral `commit_checkpoint` path; a summarizer outage degrading to the evidence index instead of losing the compaction; a summary that cannot be charged persisted (skipped-or-priced); a cost-cache write that cannot land; a setting write that cannot land; a store that cannot accept the event batch (durable append, flush, and a later append after the shared health flag was cleared); and a worker that dies with an empty batch and with a pending one. **This revision:** activation failing *after* the caller's durable commit, asserted at both layers — the direct call (error returned, registry and journal revision unchanged, transcript still readable) and the RPC surface (`fork`/`clone` report `committed but could not be activated`, keep the committed child, and replay idempotently), plus a checkpoint whose durable commit cannot land (the callback returns the error and writes nothing) |
| `concurrency` | cancel-before-first-poll beats a 30 s timeout with the child killed; Windows job/child termination on timeout; `delete_session` mutually exclusive with in-flight compaction without leaving `deleting`; `repeated_entry_reads…` (two sequential reads against the revision-keyed cache); pre-existing lagged/closed broadcast-stream tests in `grpc/mod.rs`; previous revision: a persistence worker parked by a barrier and then disconnected, with the writer's own `Drop` proving the retirement (no sleep as a synchronisation primitive). **This revision:** the escalation callback's blocking decision loop, answered from another thread through the session's own gate (the run wiring as requester, the GUI's side as decider); the cold-activation announcement is asserted on the shared global stream while other tests broadcast on it concurrently (unrelated events skipped, `Lagged` tolerated) |
| `property` | table-driven input spaces (7 thinking levels, 9 argument shapes); SQLite round-trip write → `list_installed_skills` → `uninstall_skill` → empty; idempotence (double uninstall); read-only rule injection reaches the live rule set and appends rather than replaces; equal responses across repeated reads + cache presence; **this revision:** activation idempotence (the second activation returns the same `Arc` and re-announces nothing) and fork/clone commit idempotence under a failing activation (same child id, `created:false`, entry count unchanged) |
| `platform-cfg` | Windows-only shell timeout/job branch covered **on Windows**; the `cfg(unix)` counterparts remain `platform-unmeasured` here and are authoritative on Linux/macOS CI; `export_html` success is now platform-neutral instead of `cfg(not(windows))`; path assertions compare `canonicalize_lenient` forms so the verbatim spelling cannot make them platform-dependent; **this revision:** the escalation/sandbox callbacks are covered by invoking the production closures directly, precisely because their runtime invocation needs a sandbox backend this host does not have — the lines are covered here without claiming the host can run them |
| `serialization` | typed `ContextMessage` projection (camelCase `runId`/`blocks`/`metadata`, run id lifted out of metadata, other keys preserved); `MessageBlock` `text`; skills `InstalledSkill` JSON (`id`/`scope`/`source`/`version`/`location`); `SyncResult` four-array shape; `RpcResponse` envelope `error_code`/`error_data` (**snake_case on the wire** — asserted, see F3); pre-existing `typed_payload_encodes_real_read_command_envelopes` still covers typed-payload encoding; previous revision: legacy journal `data` shapes (string-encoded vs object vs absent vs non-object) are read without aborting the replay, and a checkpoint/ticket round-trip is asserted through the real SQLite journal. **This revision:** the checkpoint the wiring persists is decoded back out of the journal (`checkpoint_id`, `summary[0].text`), and the `session_created` / `approval_request` / `approval_decision` / `tool_sandboxed` payloads are parsed as JSON and asserted field by field |

All six dimensions have real evidence; none is `N/A` for this subtree. The only unmeasurable area is
other-platform `cfg` code (`platform-unmeasured`, §6).

## 5. Weak tests repaired

All in `agent/src/rpc/session.rs`; each asserted nothing ("should not panic" / "verify no panic") or less
than the code's contract. None was deleted without a strictly stronger replacement.

| old test | replacement |
|---|---|
| `add_session_rule_does_not_panic` | `add_session_rule_records_a_read_only_allow_and_reaches_the_live_rule_set` — decision, read/write access, live rule-set layer, append-not-replace |
| `set_permission_level_invalid` | `…_value_is_stored_verbatim` — stored value + read-back |
| `set_system_prompt_updates` | asserts `loop.system_prompt` **and** the per-run `config.system_prompt` copy |
| `append_system_prompt_does_not_panic` | asserts `"base\nappended"` and no leading newline on an empty prompt |
| `set_tools_filters` | `set_tools_keeps_only_the_requested_catalogue_entries` — filtered names + unknown name ⇒ empty |
| `disable_tools_clears` | seeds tools, then asserts non-empty→empty |
| `disable_builtin_tools` | same seeding/assertion pattern |
| `fork_does_not_panic` + `delete_session_does_not_panic` | merged into `fork_and_delete_leave_the_live_session_untouched` — `Ok` + unchanged history |
| `set_sandbox_policy_updates` | asserts `None` → `Off` → replaced by `Sandbox` |
| `set_thinking_level_updates_field` / `set_thinking_level_off` | replaced by the 7-row budget table |
| `set_ephemeral_toggles` | removed: assertion-free duplicate of the asserting `set_ephemeral` |

No `#[ignore]`/`#[should_panic]` was added or relied on in this subtree.

### Repaired in the previous revision

| old state | why it was weak | repair |
|---|---|---|
| `compact_checkpoint_commit_failure_broadcasts_and_errors` asserted only `result.is_err()` | the session was never persisted, so the compaction claim failed with "session does not exist" before the summarizer or the commit ran — the test passed on the wrong error and its name was fiction | the transcript is persisted, so the claim succeeds and the assertion names the injected commit failure; it now also asserts the fence was released, `compaction_failed` (and not `compaction_committed`) was broadcast, and no checkpoint entry reached the journal |
| `compact_provider_failure_broadcasts_and_errors` asserted only `is_err()` | same claim failure; and the contract it claimed (a provider failure ⇒ error) is not the contract the code implements (F9) | renamed `compact_provider_failure_degrades_to_the_evidence_index`, persists the transcript, asserts the evidence-only outcome with the provider error as the reason, the durable checkpoint, the zero charge, and committed-not-failed |
| `event_batch_writer_refuses_appends_when_unhealthy_or_when_its_worker_is_gone` used `send(Shutdown)` + `sleep(200ms)` before asserting the enqueue failure | the sleep was a race: 200 ms only *usually* outlives the worker's exit, and a slower box would flake | replaced with `stop_worker_for_test()`, which performs the real shutdown handshake and **joins** the thread, so the worker is provably gone before the failing append |

### This revision (`todo_a6074c81c7c9`): no test was repaired, and none needed it

The seven new tests were written to fail on a wrong implementation, not to visit lines; each one names a
concrete observable and was checked against the code path it claims:

* the activation tests assert the caller-visible error **and** the durable/registry state around it (a test
  that only asserted `is_err()` would pass on an unrelated failure — the injected message is asserted verbatim);
* the cold-activation test asserts an announcement *arrives*, then that a second activation announces
  **nothing** (a test that only asserted the first event would pass with a duplicate-announce regression);
* the checkpoint test asserts the journal entry (id + type + decoded body) *and* that a failed commit writes
  nothing new, so it cannot pass if the callback swallowed the error;
* the escalation/sandboxed tests assert the published payloads field by field, so a wrong session id or a
  dropped command fails them.

Two things were deliberately *not* done: no assertion was weakened anywhere (the only pre-existing test text
this revision touched is `run_fixture`, split into a delegating wrapper so a second fixture can choose its
session id — the delegated path is byte-identical in behaviour), and no `#[cfg(test)]`-only assertion was
added to production code.

## 6. Waiver ledger — every remaining uncovered file, with category and reason

Line numbers are from the report revision (§2). Categories follow `docs/architecture/channels-test-coverage.md`
as restated in `.future/cov100/plan.md`. **Confidence** says how a reviewer should treat the row: `high` =
the arm cannot be executed here at all; `medium` = reachable in principle but only through a real external
service or a production injection seam that does not exist today — these are the rows to challenge first, so
they are called out rather than buried.

| file : lines | category | reason (and what proves the neighbours ran) | confidence |
|---|---|---|---|
| `agent/src/rpc/commands/mod.rs` : 368 | unreachable-by-construction | the final `_ =>` "unknown command" arm. The first guard in `handle_command_internal` already rejects every name not in `future_rpc::command_policy::command_policy`, and a scripted diff of that table against `commands/mod.rs` shows **all 77** real commands have a routing site; the only policy string without one is `"unknown"`, which the policy function itself rejects (`packages/rpc`'s `long_work_is_not_mistaken_for_a_long_unary_rpc` asserts `command_policy("unknown").is_none()`). `dispatcher_tests::unknown_command_returns_error` covers the outer guard. | high |
| `agent/src/rpc/commands/skills.rs` : 27 | unreachable-by-construction | `_ => unreachable!()` in the skill handler: the dispatcher routes exactly the five skill `cmd_type`s, so no other value can reach this function. | high |
| `agent/src/rpc/commands/providers.rs` : 566 | unreachable-by-construction | `serde_json::to_value` cannot fail for `SuggestSkillPayload { skill: Option<SkillCandidatePayload { String, String }> }` — no layout of two strings produces a serialization error; the code comment says the same. | high |
| `agent/src/rpc/run_snapshot.rs` : 151 | unreachable-by-construction | `let Value::String(text) = … else { unreachable!() }` in `coalesce_deltas`: a pending entry is only created from an event whose `text` was already checked `is_string`, and nothing rewrites it in between. `tool_start_arguments_are_only_significant_when_present_and_non_empty` plus the pre-existing coalescing tests exercise both sides of that filter. | high |
| `agent/src/rpc/commands/observability.rs` : 237 | attribution-artifact | line 237 is the `id,` argument inside the multi-line `RpcResponse::build_fail_code(...)` for `run_snapshot_too_large`. **`observability_tests::get_run_snapshot_rejects_unknown_and_explicitly_falls_back_for_empty_or_oversize` proves the surrounding code ran** — it pushes a snapshot over `EVENTS_PAGE_BYTE_BUDGET` and asserts `error_code == "run_snapshot_too_large"`, the code that this very call produces. | high |
| `agent/src/rpc/commands/run_control.rs` : 250 | attribution-artifact | line 250 is the `session_id = %sess.session_id,` field of the `tracing::info!` expansion in `handle_abort`. **`run_control_tests::abort_works` and `abort_session_on_idle_session_cancels_nothing` both call `handle_abort` and assert its effect**, so the statement executed; llvm-cov attributes the macro's span-end field line as uncovered. | high |
| `agent/src/rpc/prompt_helpers.rs` : 728 | platform-unmeasured | line 728 is `PathBuf::from("/workspace")`, the **unix arm of the `cfg!(windows)` in the test helper `workspace()`** (test code, not production). This measurement runs on Windows, so only the `C:\workspace` arm is compiled/executed. Authoritative platform: the Linux/macOS CI legs. | high |
| `agent/src/rpc/session.rs` (`#[cfg(unix)]` bodies) | platform-unmeasured | e.g. `execute_shell_timeout_kills_the_process_group`, `execute_shell_bounds_inherited_pipe_lifetime` and the `process_group(0)`/`killpg` production branch are `#[cfg(unix)]`: absent from a Windows report entirely (neither 0% nor denominator — `verify.py blindspot future-agent` lists them). Authoritative platform: Linux/macOS CI. | high |
| `agent/src/rpc/approval.rs` (`cfg(unix)` branches) | platform-unmeasured | same: unix-only sandbox/approval branches are not compiled here. Authoritative platform: Linux/macOS CI. | high |
| `agent/src/grpc/mod.rs` : 43–52 | unreachable-in-this-environment | `serve_local` binds the per-user IPC endpoint and serves with `serve_with_incoming_shutdown(incoming, std::future::pending())` — the shutdown future is designed never to resolve, so a test can only leak a live server for the whole process, and the agent singleton lock forbids a second instance. The TCP sibling (`serve_tcp_with`, with a real shutdown) **is** covered by `grpc::tests::serve_with_shutdown_completes_cleanly` and `session_created`-stream tests. | high |
| `agent/src/grpc/mod.rs` : 1171–1173 | attribution-artifact | the `flush` body of the `Capture: std::io::Write` test double in `grpc::tests::verbose_logs_background_commands_at_trace_only`; that named test drives the same impl's `write` (the assertion is on the captured `[grpc] get_agent_info` line) and `MakeWriter` never calls `flush`. | high |
| `agent/src/rpc/commands/settings.rs` : 13–58 | unreachable-in-this-environment | the probe-failure/serialization arms of `probe_sandbox` / `probe_windows_sandbox` / `reset_windows_sandbox`: they require `crate::sandbox::platform_sandbox_probe_product()` (resp. the Windows/reset variants) to return `Err`, i.e. a host whose sandbox probe is broken, and there is no injection seam for the probe. The success path is covered by the pre-existing `probe_*` tests. Authoritative platform: a CI runner with no sandbox host installed. | medium |
| `agent/src/rpc/commands/settings.rs` : 278–299 | unreachable-in-this-environment | the `spawn`-failure arms of `handle_compact`: `spawn` is a `std::thread::Builder::spawn` result, so the arm needs OS thread creation to fail (resource exhaustion). The succeeding path is covered by `settings_tests` compaction tests. | medium |
| `agent/src/rpc/commands/settings.rs` : 500–514, 646 | unreachable-in-this-environment | the "sandbox tier requested but the probe failed" fallback and the parent-session persistence warn: both sit behind the failing-probe condition above. | medium |
| `agent/src/rpc/approval.rs` : 220–273 | unreachable-in-this-environment | the Windows capability-approval flow (`windows_request::prepare` → `ask_user` → Approved/Cancelled/Rejected) needs a Windows sandbox host that actually demands a capability approval for the command; on this host `prepare()` reports no approval needed, so the branch is never entered. The surrounding `ask_user` machinery is covered by the pre-existing escalation tests. Authoritative platform: a Windows runner with the sandbox host installed. | medium |
| `agent/src/rpc/approval.rs` : 812, 837, 842 | unreachable-in-this-environment | three `shorten_home`/quoted-path branches needing `$HOME` itself as the command path, an unterminated quote, and a `~/`-prefixed target respectively; each is reachable only with a command shape the tool-call parser must first accept. | medium |
| `agent/src/rpc/protocol.rs` : 513, 729, 735, 737, 1941 (5 statement lines; 22 on llvm-cov's line metric — see the note below the table) | unreachable-by-construction (513, 735, 737) / unreachable-in-this-environment (729) / attribution-artifact (1941) | **513** is the `unwrap_or_else` fallback of the Flush reply: every `false` return from `flush_event_batch` is immediately preceded by `health.fail(...)`, so `health.error()` is always `Some` there — the closure body cannot run, and the reachable half of that line **is** covered (`event_batch_writer_refuses_durable_writes_after_a_failed_batch` asserts the store error it returns). **735/737** is the *second* identical "journal is permanently owned by session" guard in `configure_journal`; the first guard already rejects a single-threaded rebind, so the second needs two threads to rebind between the two checks — no deterministic seam exists and adding one would be a production change for a test. **729** is the `?` on `EventBatchWriter::new(...)`, i.e. OS thread-creation failure. **1941** is the `Ok(result)` arm of a race-tolerant helper *inside a test*. The batch-writer lines this task targeted are covered: 430 (`recv` disconnected) and 455 (`recv_timeout` disconnected) by `event_batch_worker_exits_when_its_last_sender_disappears`; 461–467 (durable append refused after a failed batch) and 512–514 (flush with a pending event) by `event_batch_writer_refuses_durable_writes_after_a_failed_batch`; the snapshot read path by `a_failed_batch_commit_is_reported_and_blocks_the_read_path`. | high |
| `agent/src/rpc/session.rs` : 830, 883, 961, 1100, 1215, 1221, 1226, 1254, plus 797/798 and the test-module lines 2262, 2298, 3166, 4400, 4437, 4573, 4852 (20 on llvm-cov's line metric — see the note below) | attribution-artifact (all), with 893 and 1221 additionally unreachable-by-construction | **The compaction-ticket arms and the usage-persistence arm of this row are now covered** and have been removed from the ledger: the ticket `finish`/`fail` pair on a failed commit (`manual_compaction_receipt_failure_is_remembered_and_not_retried`), the no-op `Unchanged` ticket (`manual_compaction_with_nothing_left_to_compact_fails_its_receipt`), `commit_checkpoint` on the ephemeral path (`ephemeral_session_checkpoint_commit_failure_is_reported`), and the "summary usage could not be persisted" arm (`a_usage_write_that_cannot_land_fails_the_compaction_receipt` — a session whose stored `session_info` is not an object lets the claim succeed while the usage merge fails, which is exactly this arm's precondition) — each asserting the observable outcome, not the line. Nothing here is `medium` any more, and no injection seam beyond the pre-existing ones was invented: the arm above needed only a store shape, not new production code. What remains is all attribution, not reachable logic. **893** is the `_ => None` arm of the checkpoint `summary` filter: a freshly prepared checkpoint's summary blocks are built by this code and are always `Text`, so no non-text block can reach the fresh-commit path (a *restored* checkpoint is read by the same arm in `compaction/mod.rs`, which is exercised). **1221** is the `continue` for a payload that fails `serde_json::from_str`: the `run_events.payload` column is written only by `append_event` (which serialises a `Value`) and carries `CHECK(json_valid(payload))`, so an unparseable payload cannot exist. **1100/1106** are the "both pipe readers finished" condition and its brace in `execute_shell_at`'s poll loop: the loop can only leave with a status through `break status` (1103-1104), which requires that condition to be true — `windows_shell_reports_exit_status_and_timeout_kills_descendants` and `shell_timeout_is_clamped_to_the_documented_bounds` assert exactly that exit status and captured output, so the statement ran. **797/798** are the argument lines of a `tracing::info!` whose statement ran 4 times; **883/961/1226/1254** are brace/region ends after converging arms, and 2262/2298/3166/4400/4437/4573/4852 are lines in the `#[cfg(test)]` module that llvm-cov attributes to the zero-count `panic!` arm of a passing `assert!` (or to the untaken `cfg!` arm at 2245) — the named tests listed in §3 prove the surrounding statements ran. | medium (825–830) / high (the rest) |
| `agent/src/rpc/mod.rs` : 312, 318, 418–445, 453, 455 on the LCOV view (31 lines; 34 on llvm-cov's line metric — see the note below) incl. the test-module lines 1307, 1310, 1312, 1316, 1338–1348 | attribution-artifact (312, 318, 1307–1348) / unreachable-in-this-environment (418–445, 453, 455) | **The post-commit `activate_persisted_session` failure this task named is covered and has been removed from this row** (the `#[cfg(test)]` hook of §3a.3 plus the four named tests of §3; its caller-visible arms in `commands/session_lifecycle.rs` went with it). **This also corrects the previous revision's claim**: the lines it attributed here (the old 342–343 “activation failure/warn arms”) were branch/region artifacts — the activation statements were already line-covered by the pre-existing fork/clone tests (F15), which is why this file's uncovered count did not move while two other files dropped by 30. What is genuinely left: **312/318** is `create_session`'s foreign-journal arm — `create_session_rejects_a_foreign_broadcaster_without_rebinding_it` enters it and asserts a replacement broadcaster was installed; the unexecuted statement in that arm is the `tracing::error!` for a failed `configure_journal` (needs a store that refuses a journal for a brand-new broadcaster, and the code deliberately continues with the fresh broadcaster, so only a log line is at stake). **418–445, 453, 455** is `reconcile_provider_references`: the “no configured replacement model” warn + `continue` (a registry whose `replacement_model` returns `None` — this host always has a builtin default) and the “session configuration is busy” retry loop / persist-failure arm (needs another thread holding the short config lock, or a failing `set_model`); no seam exists and adding one would be a production change made only for a test. **1307–1348** are in the `#[cfg(test)]` module: the zero-count `panic!` arms of passing `assert!`s and the `Lagged`/`Closed` drain arms of the global-stream tests. | medium (418–445, 453, 455) / high (the rest) |
| `agent/src/rpc/session_prompt.rs` : 1063, 1470 on the LCOV view (2 lines; 5 on llvm-cov's line metric) | attribution-artifact | **The three run callbacks this task named are covered and have been removed from this row**: the `#[cfg(test)]` capture of §3a.4 hands a test the production closures, and the three tests of §3 assert their effects (a durable checkpoint entry with its decoded body, and a failed commit that returns the error and writes nothing; the published `approval_request`/`approval_decision` exchange with a decision returned from the session's gate; the `tool_sandboxed` event for the exact command). What remains are two closing braces / span ends of the wiring, whose surrounding statements the named tests ran — the file went 27 → 5 uncovered. **Caveat (F16):** `on_checkpoint` is *not* reachable at runtime in this wiring (the compaction journal is installed under the same `!is_ephemeral` condition and its ticket always wins the commit), so its coverage comes from invoking the captured production closure, not from a live compaction. | high |
| `agent/src/rpc/commands/session_lifecycle.rs` : 278, 546 on the LCOV view (2 lines; 4 on llvm-cov's line metric) | attribution-artifact | **The fork/clone post-commit activation arms are covered and have been removed from this row** (`fork_reports_a_post_commit_activation_failure_and_keeps_the_committed_child`, `clone_reports_a_post_commit_activation_failure_and_keeps_the_committed_child`, plus `activate_persisted_session_reports_a_post_commit_failure_and_keeps_the_session_readable`): the file went 12 → 4 uncovered. **278** is the `unwrap_or("")` default of `c.as_str()` in `cmd_get_fork_messages`' bare-content arm — the arm itself is covered by `get_fork_messages_handles_legacy_bare_string_content`; the default needs a fork-point content that is neither an array nor a string (a number/object), a shape the reader filters out. **546** is the closing brace of the display-cache block in `cmd_read_session_entries`, whose statements `repeated_entry_reads_are_served_from_the_cached_projection` exercises. | high |
| `agent/src/rpc/commands/session_title.rs` : 521–525 | unreachable-in-this-environment | the success arm of `handle` (`Ok(data) => RpcResponse::ok(...)`): it needs a real LLM to answer with a title. `a_real_title_request_selects_the_configured_provider_and_fails_closed` now covers everything up to and including the request (model resolution, target construction, provider client, runtime dispatch); only the returned title is missing. Authoritative platform: an integration run against the real provider. | medium |
| `agent/src/rpc/commands/session_lifecycle.rs` : fork/clone activation arms | **covered this revision — see the `session_lifecycle.rs` row above** (this line is kept only so a reader of an older revision finds the redirect). | — |
| `agent/src/rpc/session.rs` : compaction ticket arms, cost fallbacks, history-merge helper (73 lines) | unreachable-in-this-environment | **superseded by the row above** (this line is kept only so a reader of an older revision finds the redirect): the ticket arms are covered as of this revision; the remaining lines and their categories are in the `agent/src/rpc/session.rs` row. | high |
| `agent/src/rpc/commands/providers.rs` : 559–561 | unreachable-in-this-environment | the `SkillCandidatePayload` mapping in `cmd_suggest_skill` runs only when `skill_reco::suggest_skill` returns a candidate, which requires a signed-in, in-budget Jev service call; here it always returns `None` and the handler correctly answers `{"skill": null}`. | medium |

### Note on the two line views (why the two numbers differ)

llvm-cov's `Lines` metric (§2, and the gate's basis) counts *every* line whose region set is
uncovered — including the zero-count `panic!` arm of a passing `assert!` and brace ends after a converging
arm. The LCOV/cobertura exports of the same run only emit `DA:` records for a subset of those lines, so the
line numbers quoted in the rows above come from the LCOV export (`cargo llvm-cov report --lcov`,
`coverage/ag-rpc6.lcov`) and are a *subset* of the counted lines: **31 vs 34** for `mod.rs`, **2 vs 5** for
`session_prompt.rs` and **2 vs 4** for `commands/session_lifecycle.rs` (a revision ago: 18 vs 20 for
`session.rs` and 5 vs 22 for `protocol.rs`). The plan's warning applies here in the other direction as well:
do not recount uncovered lines from either export — §2's `uncovered` column is the authoritative figure, and
the LCOV numbers are for locating code. The named tests in §3 are what prove the attribution claims; a
zero-count region on a line is not evidence that the line's statement did not run (F15 is the worked example
of that mistake).

### Report blind spots (sources under the subtree with no measured lines)

`verify.py` warns that 9 of the 26 sources under `agent/src/rpc/` and `agent/src/grpc/` are absent from the
report; six are already named in §3. The remaining three, all `#[cfg(test)] mod *_tests;` units (they do not
exist in a non-test build, so they cannot hide production code, and llvm-cov emits no entry for them):

| file | category | why it contributes no data |
|---|---|---|
| `agent/src/rpc/commands/providers_tests.rs` | attribution-artifact | `#[cfg(test)]`-only module of tests for `commands/providers.rs`; its lines are counted inside no file entry of the export (the same is true of its sibling `dispatcher_tests.rs`/`settings_tests.rs`, which §3 happens to name) |
| `agent/src/rpc/commands/run_control_tests.rs` | attribution-artifact | same: `#[cfg(test)]`-only, covering `commands/run_control.rs` |
| `agent/src/rpc/session_prompt/build_user_message_tests.rs` | attribution-artifact | same: `#[cfg(test)]`-only, covering `session_prompt`'s message builder |

The other six are `commands/{dispatcher,observability,skills,settings,session_lifecycle}_tests.rs` and
`session_prompt/tests.rs`, already named in §3's file table. Nothing here understates coverage of a
*production* line: every production source of the subtree appears in the report.

### Rows a reviewer should challenge first

This revision emptied the three rows that were called out here last time:
`agent/src/rpc/mod.rs` (post-commit activation failure), `agent/src/rpc/session_prompt.rs` (run callbacks) and
`agent/src/rpc/commands/session_lifecycle.rs` (fork/clone activation arms) — all three now carry the named
tests of §3. The activation row also had to be *corrected*, not just emptied (F15). What is left to attack,
cheapest first:

1. `agent/src/rpc/mod.rs` 418–445 — the reconciliation arms. They are real, reachable code with no seam today
   (a `Registry` with no replacement model; a concurrent holder of the short config lock). A reviewer who
   builds the seam this file already demonstrates (`RELOAD_RACE_HOOK`) could move them.
2. `agent/src/rpc/commands/settings.rs` (probe failure — an environment statement, not a seam: try it on a
   runner with no sandbox host installed) and `agent/src/rpc/approval.rs` (Windows capability approval on a
   provisioned host — same).
3. `agent/src/rpc/commands/session_title.rs` (a real provider answer) and `agent/src/rpc/commands/providers.rs`
   559–561 (a signed-in, in-budget Jev service call).

## 7. Findings

* **F1 — `cargo llvm-cov report` alone is unsafe to *measure* with, in a shared checkout.** After a run
  whose test target fails, the profiles are consumed and a standalone `report` merges stale/foreign
  `.profraw` → an all-zero report (`0/60903`, caught by the gate's "broken measurement" check). Always
  measure atomically. (A `report --lcov` read of a *successful* run in a private target dir is safe and is how
  §6's line numbers were obtained; it plays no part in §2/§8.)
* **F2 — the checkout is not stable enough for a whole-crate measurement.** During this segment the crate
  failed to compile (`agent/src/sandbox/rules.rs` `Vec::len` mismatch, `agent/src/tools/mod.rs` unterminated
  character literal, `agent/src/session/repair.rs` `&str`/`String`) and the failing set rotated between runs
  (`session::database`, `session::display`, `session::fork`, `session::legacy_import`, `models`, `tools`).
  None is in this subtree. `models::tests::registry_injects_future_models_from_disk_cache` passes 68/68 in
  isolation (`cargo test … -- models::tests::`) and fails only inside the full parallel suite — pre-existing,
  owned by `agent/src/models/`, not fixed here (out of scope). **This revision's run was green**: one
  whole-`--lib` measurement run of **2012 tests** completed with 0 failures (one pre-existing `#[ignore]`),
  which is why §2's report is the unfiltered one. The crate also compiles clean for this subtree's test
  target; the shared checkout remains a moving target for *other* reasons — see F14.
* **F14 — the shared checkout still carries another worker's in-flight lint/format state.**
  `agent/src/llm/adapters/anthropic.rs:1233` has an unused, needlessly-`mut` local (two `rustc` warnings), and
  `cargo fmt -p future-agent --check` reports unformatted hunks in `agent/src/cli.rs`,
  `agent/src/config/providers.rs`, `agent/src/llm/adapters/anthropic.rs`, `agent/src/session/persistence.rs`,
  `agent/src/session/sqlite_store.rs` and others outside this subtree. Both would fail CI (`-D warnings` /
  `--check`) but are not this task's files: they are reported, not touched. The four files *this* subtree owns
  (`protocol.rs`, `session.rs`, `commands/skills_tests.rs`, `commands/observability_tests.rs`) were formatted
  with `rustfmt --edition 2021` and are now `--check`-clean. This revision adds `mod.rs`,
  `commands/session_lifecycle_tests.rs`, `session_prompt.rs` and `session_prompt/tests.rs` to the list of
  files whose formatting this task owns and checked (`rustfmt --edition 2021 --check`: all four clean), and
  removed the one clippy error inside the subtree (`session.rs` `clippy::useless_vec`, §3). `cargo clippy -p
  future-agent --all-targets -- -D warnings` still fails, but **only** on the three files named above
  (`cli.rs:69`, `llm/adapters/anthropic.rs:1236`, `session/database.rs:888`) — all outside this task's
  declared write set, reported and not touched.
* **F3 — `RpcResponse` error fields are snake_case on the wire** (`error_code`, `error_data`; no
  `rename_all`). CamelCase assertions fail; the canonical spelling is asserted by `protocol.rs`'s own tests.
  A client using camelCase would silently read `null`.
* **F4 — `SandboxTier` compared as a bare string.** `settings.rs` tests `requested_tier == "sandbox"` while
  the rest of the code uses the enum; a rename would silently disable the fallback branch (and it is one of
  the rows I could not cover). Not a defect today.
* **F5 — `TestHome` is load-bearing for skill tests.** `SkillManager::local()` resolves through the FutureOS
  home, so a skill test that forgets `TestHome` writes into the developer's real `~/.future`.
* **F6 — pre-existing liveness deadlines are too tight under instrumentation** (fixed here, in this
  subtree): `session.rs::model_context_downshift_defers_compaction_until_llm_preflight` used a 2 s timeout
  and failed with `Elapsed(())`; widened to 60 s with a comment saying the assertion is liveness, not speed.
  The claim ("selecting a model does not compact until an LLM request") is unchanged.
* **F7 — same class, fixed here**: `session_prompt/tests.rs::wait_for_run_end` polled 500 × 10 ms = 5 s and
  panicked "run did not finish within 5s"; widened to 3000 × 10 ms = 30 s (message updated to match), because
  the instrumented parallel run of the whole crate is several times slower than a single-test run.
* **F8 — one flaky test remains, not mine, not fixed**: `grpc::tests::verbose_logs_background_commands_at_trace_only`
  intermittently fails with an **empty capture buffer** (the panic message is literally empty), while passing
  in isolation. `agent/src/grpc/mod.rs` is **unmodified** in this branch (`git status` shows no change; last
  touch is commit `cb90f5da`), and the test registers its subscriber with `with_default`. Nothing else in the
  crate calls `set_global_default`; the empty buffer points at `tracing`'s thread-local subscriber /
  callsite-interest behaviour under concurrent subscriber registration elsewhere in the suite. It did **not**
  fire in the final measurement run, so the report includes it. It should be diagnosed by someone who can
  change that test (the fix is likely to assert on a signal the test owns rather than on a thread-local
  capture).
* **F9 — a test was passing on the wrong error, and its name was wrong too (fixed in the previous revision).**
  `compact_provider_failure_broadcasts_and_errors` never persisted its session, so the compaction claim
  failed with "session does not exist" before the summarizer ran; the assertion (`is_err()`) held anyway.
  Persisting the transcript exposed the real contract: a summarizer that *errors* does **not** fail the
  command — `prepare_with_handoff_summary` degrades to the deterministic evidence index and commits it with
  `summaryOutcome.status = "evidence_only"` and the provider error as `fallback_reason`. The same class of
  false pass applied to `compact_checkpoint_commit_failure_broadcasts_and_errors` (same missing claim). Both
  are rewritten with the claim succeeding; see §5.
* **F10 — per-file coverage of an untouched file moves between identical runs of untouched code.**
  This revision produced the cleanest measurement of the effect yet, because it ran the *same* command twice on
  a tree that differed only by a token-level clippy fix in one test: `agent/src/rpc/commands/settings.rs`
  measured **52 uncovered (535/587) then 55 (532/587)**, and `agent/src/rpc/session.rs` **19 then 20**
  (`LF` 4021 vs 4019 — the fix's array literal accounts for at most 1 of those lines). Both files are
  untouched by this task and neither delta can come from the code. The previous revision saw the same ±3 on
  `settings.rs` (52 ↔ 55) and a ±1 on `mod.rs`. Its uncovered set is dominated by probe/environment branches,
  so the cause is a load- or order-dependent branch (a cached sandbox probe) rather than a code change — a
  3-line movement on this box is not evidence of anything. **What is *not* noise**: the two files this
  revision targeted moved by −22 and −8 in *both* passes, and the three target files' `DA:0` line sets are
  identical in both LCOV exports.
* **F11 — llvm-cov's line metric and its own LCOV export disagree on this subtree.** For the same run the gate
  counts 34/5/4 uncovered lines in `mod.rs`/`session_prompt.rs`/`session_lifecycle.rs` (and 20/55 for
  `session.rs`/`settings.rs`), while `--lcov` emits 31/2/2 `DA:0` records (and 18/53) — the same held a
  revision ago (`session.rs` 20 vs 18, `protocol.rs` 22 vs 5). The gate's figure is the authoritative one and
  §2 uses it; §6 quotes LCOV line numbers only to locate code (the export quoted is regenerated from the final
  pass' profile data, so its line numbers match this revision's tree). A reviewer who needs exact line sets
  should use the LCOV/cobertura export of the same report revision and not the JSON `segments`. **Direction of
  the error matters for the *seam* rows**: for the three rows this revision emptied, the LCOV view is *more*
  favourable (fewer lines) than the gate's, so the claims in §6 rest on the named tests and their assertions,
  not on either line list.
* **F12 — the compaction usage-persistence arm needed no new seam after all.** The previous revision judged
  `session.rs` 825–830 (`ticket.fail` when the summary usage cannot be persisted) "medium: needs a
  `SessionPersistence` failure hook". It is reachable with the *existing* hooks: the claim in `admit` only
  needs the session row, and `update_session_info_fields` fails when the latest `session_info` content is not
  an object — a store shape a test can create with `manager.save`. Covered by
  `a_usage_write_that_cannot_land_fails_the_compaction_receipt`. No production hook was added, and
  `agent/src/session/` remains untouched.
* **F13 — a schema constraint makes one arm unreachable by construction.** `agent/src/rpc/session.rs` 1221
  (the `continue` for a journal payload that fails `serde_json::from_str`) cannot run: `run_events.payload`
  carries `CHECK(json_valid(payload))` (`agent/src/session/database.rs`) and is written only by
  `append_event`, which serialises a `serde_json::Value`. That is a property of the store's schema, not of
  this code, so the arm should be kept as defence in depth and waived rather than deleted.
* **F15 — the previous revision's `rpc/mod.rs` row was a region-view artifact, and the real gap was in the
  callers (corrected here).** The row claimed `activate_persisted_session`'s "failure/warn arms" needed a
  seam to reach. In the code there are only two failure expressions in that method, the `?` on
  `try_get_session` and the `ok_or_else`, and both were **already line-covered** by the pre-existing
  fork/clone tests (a fork of a deleted parent, etc.). What was actually uncovered was the *callers'* handling
  of a post-commit activation failure (`commands/session_lifecycle.rs` 602–606 and 653–657), which this
  revision covers. That is why `mod.rs`'s uncovered count is flat (34 → 34) while the two files the seam
  drives dropped by 30 lines together: the 9 instrumented lines the new `#[cfg(test)]` seam adds are all
  covered, and no production line's status changed. **Lesson for a reviewer chasing a region list**: a
  zero-count region on a line is not evidence that the statement on it did not run.
* **F16 — the session wiring's `on_checkpoint` callback is shadowed by the compaction ticket (found here).**
  `prompt_internal` installs `on_checkpoint: (!is_ephemeral).then_some(checkpoint_callback)` and
  `compaction_journal: (!is_ephemeral).then(|| …)` under the same condition. In `run_loop.rs` (both the
  automatic site and the provider-limit recovery site) the commit is
  `if let Some(ticket) = &compaction_ticket { ticket.finish(…) } else if let Some(commit) = &ctx.on_checkpoint { commit(…) }`,
  and `prepare_with_journal_and_summary` returns `Some(ticket)` for **every** admission when the journal is
  present (`Replayed` always, `Fresh(Some(_))` once `admit` claims an operation; `Fresh(None)` only when no
  journal was passed). So in this wiring the callback can never be called at runtime, and the checkpoint is
  always committed through the ticket instead. This is not a defect — the checkpoint does get committed, and
  the ticket path is tested — but it means the callback's coverage must come from invoking the closure
  directly (which this revision does, with a real checkpoint and a real persistence boundary) and that the
  field is dead weight in this wiring. Removing it would be a production change outside this task's remit
  (“if a place can only be done by changing production behaviour, stop and report”), so it is **reported
  here** rather than touched; a follow-up could either delete the callback or make it the actual commit path
  for the ephemeral/journal-less case (`!is_ephemeral` is false there, so today the `else` branch is `Ok(())`).

## 8. Gate status — `gate-green`

The exact command from §1 was run against the §2 report:

```
future-agent/ag-rpc: 98.3961% lines (15092/15338) across 17 files in 2 dir(s),
                     246 uncovered line(s) in 16 file(s), target 99.99%
  every uncovered file (16) is waived with a category in docs/testing/module-agent-rpc.md
PASS  (exit 0)
```

The run behind that report was green end-to-end (**2012 passed / 0 failed / 1 pre-existing `#[ignore]`**), so
the number is not a partial-profile artifact.

The subtree is **not** at 99.99%, so this completion rests on the waiver half of the acceptance contract:
**all 16 files with uncovered lines carry a category-bearing row in §6**, which is what the gate's
`_waiver_gate` requires (it names every file by repo-relative path and a category on the same line). The gate
also rejects file-level `OPEN` declarations, so none remain — instead §6 marks each row's confidence and §6's
closing list names exactly which rows a reviewer should try to falsify. The failure modes are reported, not
hidden: 246 lines remain uncovered, none of them claimed as covered, and the three rows this revision was
asked to attack no longer carry a `medium` claim (the `medium` that remains sits on the reconciliation arms of
`mod.rs` and the environment rows, which this task explicitly kept out of scope).

Structural checks that also pass: every uncovered file is named with a repo-relative path, no row relies on a
bare filename, `verify.py doc docs/testing/module-agent-rpc.md 6` passes, and no test in this subtree is
`#[ignore]`d or assertion-free (`0` `#[ignore]` matches under `agent/src/rpc/`). The goal-level audit docs the
`verify.py weak` / `verify.py skipped` commands look for (`docs/testing/weak-test-audit.md`,
`docs/testing/disabled-test-audit.md`) do not exist yet; they aggregate every module and are outside this
task's declared write set, so this document states the subtree-level fact instead of creating them.

**Evidence tokens for this handoff: `lines-100-or-waived`, `dimensions`, `weak-tests-fixed`.** Reading them
against §2/§6: the subtree is not at 100% but every uncovered line is waived with a category (`lines-100-or-waived`);
all six `dimensions` have named evidence in §4 (and this revision added error-path, concurrency, property,
serialization and platform-cfg cases); and this revision repaired no weak test because it wrote none — the
`weak-tests-fixed` evidence for the module is the repair table in §5 plus the check paragraph that follows it.

## 9. Handoff summary

* **Artifacts**: this document; `coverage/agent-ag-rpc-report.json` (the gate report, this revision);
  `coverage/ag-rpc6.lcov` (per-line detail of the same profile data, `--lcov`, non-measuring);
  `coverage/ag-rpc-json-gaps.py`, `coverage/ag-rpc-gaps.py`, `coverage/ag-rpc2-lcov.py` (re-derive §2's table /
  the `DA:0` line sets); test sources listed in §3.
* **New versus the previous revision** (`todo_a6074c81c7c9`): the previous revision had built the two seams for
  `session.rs`/`protocol.rs`. This one adds the two the task named — the id-gated
  `ACTIVATE_PERSISTED_SESSION_FAIL_HOOK` in `rpc/mod.rs` (post-commit activation failure, §3a.3) and the
  run-callback capture in `session_prompt.rs` (§3a.4) — plus **7 tests** and the resulting **−30 uncovered
  lines** (276 → 246; `session_prompt.rs` 27 → 5, `commands/session_lifecycle.rs` 12 → 4), with the `mod.rs`
  row corrected rather than emptied (F15) and a new finding about the `on_checkpoint` wiring (F16).
* **Rejected**: (a) waiving reachable lines as `unreachable-by-construction` — the checks that such a claim
  would need to pass are in §6 only where the evidence exists; (b) building the report from a skip list —
  measured at 96.59%, i.e. it understated the subtree by ~1%; (c) `cargo llvm-cov report` as a separate step
  to *produce* §2 (it is used only to read the line sets of an already-produced private-dir profile); (d) a
  failure hook in `agent/src/session/persistence.rs` — outside this task's declared write set, and F12 shows
  it was not needed; (e) making `EventBatchWriter::sender` an `Option` so the writer could drop it —
  considered and rejected in favour of a seam that leaves the production field alone; (f) **this revision** —
  (i) refactoring the checkpoint closure into a private helper instead of a `#[cfg(test)]` capture (the task
  requires the seam to be `#[cfg(test)]`-gated), (ii) driving `escalation`/`on_sandboxed` through a real
  sandboxed run (needs a sandbox backend this host does not have, which is why the pre-existing test
  early-returns), (iii) arming the activation hook by a plain `Option<String>` for the *child* id of a fork
  (the id is minted inside the commit) — the tests arm the failure on the id the first call returned and
  replay the idempotent commit instead, (iv) deleting the dead `on_checkpoint` wiring (production change,
  out of remit — reported as F16).
* **Uncertainty**: (a) the remaining `medium` rows in §6 (settings.rs probe, approval.rs Windows capability,
  the `mod.rs` reconciliation arms, session_title real answer, providers.rs Jev call) are environment/seam
  statements, not proofs of unreachability — they are listed explicitly so a reviewer can overturn them;
  (b) F16 is a *reading* of the wiring plus the compiler-visible types, not an observed runtime miss — it is
  falsifiable by a test that makes `prepare_with_journal_and_summary` return `Compacted` with `ticket: None`
  while a journal is installed; (c) run-to-run movement on this box is ±3 lines per file (F10), so the
  per-file deltas of ≤1 in the untouched files of §2 mean nothing; (d) the attribution claims for brace/macro
  lines rest on the named tests in §3 running the surrounding statement, not on a line-level proof.
* **Next useful check**: (1) re-measure with the §1 command and confirm `session_prompt.rs` stays at 5 and
  `commands/session_lifecycle.rs` at 4 — a rise would mean the new tests pick up state from the parallel suite
  rather than from the seams; (2) attack the `mod.rs` reconciliation arms (the cheapest real gap left in this
  subtree) with a seam of the same shape as `RELOAD_RACE_HOOK`; (3) the §6 environment rows
  (`settings.rs` probe, `approval.rs` Windows capability) need a host with a sandbox backend, not a seam.
