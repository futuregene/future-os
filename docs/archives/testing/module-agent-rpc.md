# Archived testing records — `docs/testing/module-agent-rpc.md`

Moved out of `docs/testing/module-agent-rpc.md` on 2026-09-27 during the
historical-pollution cleanup (task T4a, goal `goal_39567c2a6b22`,
worktree `docs-testing`). The text below is verbatim, with its original
dates; the live record keeps a one-line pointer where each block was
removed.

---

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

---


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

---

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

---


| old state | why it was weak | repair |
|---|---|---|
| `compact_checkpoint_commit_failure_broadcasts_and_errors` asserted only `result.is_err()` | the session was never persisted, so the compaction claim failed with "session does not exist" before the summarizer or the commit ran — the test passed on the wrong error and its name was fiction | the transcript is persisted, so the claim succeeds and the assertion names the injected commit failure; it now also asserts the fence was released, `compaction_failed` (and not `compaction_committed`) was broadcast, and no checkpoint entry reached the journal |
| `compact_provider_failure_broadcasts_and_errors` asserted only `is_err()` | same claim failure; and the contract it claimed (a provider failure ⇒ error) is not the contract the code implements (F9) | renamed `compact_provider_failure_degrades_to_the_evidence_index`, persists the transcript, asserts the evidence-only outcome with the provider error as the reason, the durable checkpoint, the zero charge, and committed-not-failed |
| `event_batch_writer_refuses_appends_when_unhealthy_or_when_its_worker_is_gone` used `send(Shutdown)` + `sleep(200ms)` before asserting the enqueue failure | the sleep was a race: 200 ms only *usually* outlives the worker's exit, and a slower box would flake | replaced with `stop_worker_for_test()`, which performs the real shutdown handshake and **joins** the thread, so the worker is provably gone before the failing append |
