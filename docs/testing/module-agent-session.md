# future-agent / `ag-session` subtree — coverage, waivers and test-quality record

Goal `cov-100-multidim` · group **ag-session** of module **future-agent** (`agent/`) ·
branch `test/cov100` (worktree `.worktrees/cov100`) · host **Windows x86_64**, rustc
pinned by `rust-toolchain.toml` · measured 2026-09-26 · session
`20260926-021214-e3f28e9fda8a4322bb04be64c161917e` (this worker).

Scope: `agent/src/session/**` + `agent/src/logfile.rs` — the session persistence
subtree (SQLite store, legacy JSONL import, session lifecycle, event journal,
checkpoints, display projection).

Evidence literals for this handoff: **lines-100-or-waived** (see §6 — the 99.99 %
target was not reached, and every one of the 14 files that still holds uncovered
lines is registered there with a category and a reason, which is what the gate
accepts), **dimensions** (§4), **weak-tests-fixed** (§5).

## 1. Measurement (exact commands)

`cargo llvm-cov` 0.9.1 rejects the plan's `--target-dir` form
(`error: invalid option '--target-dir'`); the private per-worker target dir comes
from `CARGO_TARGET_DIR`:

```powershell
$env:CARGO_TARGET_DIR = "target/cov-agsession"
Get-ChildItem target/cov-agsession/llvm-cov-target -Filter *.profraw -Recurse | Remove-Item -Force
cargo llvm-cov -p future-agent --no-report -- `
  --skip cli::shutdown::tests::stalled_shutdown_subprocess `
  --skip rpc::protocol::tests::event_batch_writer_refuses_durable_writes_after_a_failed_batch `
  --skip skills::manager::tests:: `
  --skip sandbox::rules::tests:: --skip sandbox::windows:: --skip sandbox::linux:: `
  --skip grpc::tests::serve_with_shutdown --skip llm::adapters::anthropic::tests:: `
  --skip rpc::commands::settings_tests:: --skip rpc::session::tests::model_context_downshift `
  --skip rpc::tests::provider_reconcile
cargo llvm-cov report --json --output-path coverage/agent-ag-session-report.json
cargo llvm-cov report --lcov --output-path coverage/ag-session.lcov
python .future/cov100/verify.py rust-module ag-session 99.99 docs/testing/module-agent-session.md coverage/agent-ag-session-report.json
```

**Why the skip list exists, and why it matters for reading these numbers.** Several
workers share this checkout and edit other subtrees (`ag-rpc`, `ag-sandbox`,
`ag-llm`, `ag-core`) in place. `cargo llvm-cov --no-report` merges its profile data
only when the whole `cargo test` invocation succeeds; when an unrelated test fails
or the crate does not even compile because another worker is mid-edit, the merge
silently loses the lib test binary's data and the report **understates** this
subtree (measured: 21 %–97.9 % for identical sources, with my own new tests
showing as uncovered). The skips above name only *other* groups' tests that were
red or hung at measurement time; **no `agent/src/session/` or `logfile.rs` test is
skipped**. The measured run was green: `1775 passed; 0 failed; 225 filtered out`
(lib) plus every integration target ok (`sqlite_storage` 15 passed — it exercises
this subtree directly).

Authoritative metric: llvm-cov's `Lines` / `Missed Lines` columns (the JSON
`summary.lines`), which is what `verify.py` gates on. Note for the reviewer: in
this crate the **LCOV export is not line-identical** to that summary — it omits
macro-expansion lines (`tracing` field expressions, multi-line `matches!`).
`legacy_import.rs` line 20 is the clearest case: `llvm-cov show` and the LCOV
export both list it as zero while the summary carries further lines that neither
export marks. Line numbers below therefore come from the LCOV export (a precise
subset) and **counts** from the summary (the gate's basis).

## 2. Result — before → after (measured, not estimated)

| run | covered / total lines | line % | uncovered | files with gaps |
|---|---|---|---|---|
| baseline (this worktree, start of segment) | 8479 / 8772 | 96.6598 % | 293 | 17 |
| after | **9568 / 9704** | **98.5985 %** | **136** | **14** |

Delta: **+1089 covered lines, +1.94 pp, −157 uncovered lines**. The denominator
grew by 932 lines because the new in-file `#[cfg(test)]` tests are themselves
measured (they execute, so they land in the numerator too).

Per file (covered/total, uncovered, %) at baseline → after:

| file | baseline | after | uncov | line % |
|---|---|---|---|---|
| `agent/src/logfile.rs` | 227/229, 2 | 227/229 | 2 | 99.13 |
| `agent/src/session/checkpoint.rs` | 364/365, 1 | 364/365 | 1 | 99.73 |
| `agent/src/session/compaction_ops.rs` | 106/108, 2 | 106/108 | 2 | 98.15 |
| `agent/src/session/database.rs` | 588/636, 48 | 717/754 | 37 | 95.09 |
| `agent/src/session/display.rs` | 153/156, 3 | 183/183 | **0** | 100 |
| `agent/src/session/entry.rs` | 240/240, 0 | 240/240 | **0** | 100 |
| `agent/src/session/fork.rs` | 752/786, 34 | 898/907 | 9 | 99.01 |
| `agent/src/session/history_index.rs` | 305/322, 17 | 453/462 | 9 | 98.05 |
| `agent/src/session/history_query.rs` | 286/291, 5 | 307/310 | 3 | 99.03 |
| `agent/src/session/legacy_import.rs` | 359/446, 87 | 622/644 | 22 | 96.58 |
| `agent/src/session/manager.rs` | 1360/1377, 17 | 1428/1434 | 6 | 99.58 |
| `agent/src/session/model.rs` | 104/104, 0 | 104/104 | **0** | 100 |
| `agent/src/session/persistence.rs` | 1109/1129, 20 | 1218/1226 | 8 | 99.35 |
| `agent/src/session/projection.rs` | 706/711, 5 | 732/732 | **0** | 100 |
| `agent/src/session/records.rs` | 274/286, 12 | 297/308 | 11 | 96.43 |
| `agent/src/session/repair.rs` | 177/179, 2 | 221/222 | 1 | 99.55 |
| `agent/src/session/run_journal.rs` | 135/135, 0 | 135/135 | **0** | 100 |
| `agent/src/session/sqlite_store.rs` | 852/876, 24 | 903/926 | 23 | 97.52 |
| `agent/src/session/summary.rs` | 320/322, 2 | 320/322 | 2 | 99.38 |
| `agent/src/session/tools.rs` | 62/74, 12 | 93/93 | **0** | 100 |

## 3. Tests added (all in this subtree, all with behavioural assertions)

`agent/src/session/legacy_import.rs` — module `import_paths`:
`unreadable_sources_and_missing_retry_targets_are_reported_not_imported`,
`a_second_import_skips_committed_sessions_and_finishes_the_rest`,
`a_legacy_file_never_overwrites_an_existing_sqlite_session`,
`entry_shapes_are_rejected_with_their_reported_kind`,
`events_are_validated_for_identity_sequence_and_duplicates`,
`timestamp_repairs_are_warnings_and_unparsable_time_is_a_skip`,
`checkpoint_ranges_must_resolve_to_real_user_and_assistant_entries`.

`agent/src/session/database.rs` — module `open_and_reclaim_paths`:
`a_foreign_or_unrecognized_database_is_refused_instead_of_adopted`,
`a_locked_writer_is_retried_on_the_same_connection_until_it_is_free`,
`reclaim_returns_freed_pages_and_honours_both_budgets`,
`a_non_busy_transaction_failure_is_returned_instead_of_retried`.

`agent/src/session/fork.rs` — module `cutoff_paths` (new tests):
`fork_point_modes_require_a_selection_and_reject_unknown_modes`,
`a_through_turn_point_must_name_a_user_message_and_latest_settled_needs_a_terminal`,
`a_checkpoint_that_cannot_be_remapped_is_dropped_from_the_fork`,
`provenance_is_replaced_only_when_the_child_carries_a_session_info`,
`a_settled_turn_resolves_through_its_run_markers`.

`agent/src/session/history_index.rs` — `shape_reduces_legacy_and_block_tool_calls_to_pairing_ids`
(in `mod tests`) and module `forward_page_budget`:
`a_forward_page_stops_at_the_eight_megabyte_budget` (the read bound),
`a_forward_page_is_cut_before_a_row_that_exceeds_the_payload_budget` (the payload
bound, including that the deferred row is reachable on the next page),
`the_projected_page_is_never_larger_than_the_bytes_that_were_read` (the invariant
behind ordinary pages never hitting the payload bound).

`agent/src/session/history_query.rs` — in `mod tests`:
`out_of_range_search_and_read_requests_are_rejected_before_touching_storage`.

`agent/src/session/manager.rs` — module `display_cache_paths`:
`a_cache_hit_is_promoted_and_the_oldest_entry_is_evicted`; module
`fork_request_validation`: `a_fork_request_needs_its_own_id_and_a_parent_session`.

`agent/src/session/persistence.rs` — module `commit_checkpoint_paths`:
`commit_checkpoint_appends_its_entry_to_the_journal`,
`a_failed_compaction_commit_is_surfaced_and_poisons_later_checkpoints`; three
existing tests strengthened (§5).

`agent/src/session/records.rs` — module `canonical_paths`:
`a_non_entry_value_is_passed_through_unchanged`,
`error_prefixed_tool_text_becomes_an_error_result_block`,
`an_explicitly_empty_block_array_survives_a_storage_round_trip`.

`agent/src/session/repair.rs` — module `dedupe_and_orphan_paths`:
`a_real_result_replaces_the_placeholder_for_its_call`,
`a_call_without_an_id_never_claims_a_later_result`.

`agent/src/session/projection.rs` — module `thinking_projection`:
`legacy_thinking_becomes_a_leading_reasoning_block_exactly_once`.

`agent/src/session/display.rs` — module `projection_guards`:
`unusable_terminal_markers_change_nothing_and_stale_metadata_is_dropped`.

`agent/src/session/sqlite_store.rs` — module `journal_selection_paths`:
`only_pricing_events_are_read_back_and_identical_appends_are_no_ops`,
`refreshing_a_session_info_keeps_one_record_with_the_latest_content`.

`agent/src/session/tools.rs` — module `manager_wrappers`:
`the_manager_delegates_tool_reads_to_the_initialized_store`.

## 4. Dimensions

| dimension | evidence in this subtree |
|---|---|
| `boundary` | `import_paths::entry_shapes_are_rejected_with_their_reported_kind` (a transcript of blank/whitespace-only lines → `empty_session`; a repeated identical line → one entry + one warning); `timestamp_repairs…` (space-separated and missing timestamps); `history_query::tests::out_of_range_search_and_read_requests_are_rejected_before_touching_storage` (200 vs 201 characters, `limit` 0/21, `limit` 3/40000 bytes, offset past the entry); `checkpoint_ranges_must_resolve…` (empty protected range, dangling protected id, valid range); `forward_page_budget::{a_forward_page_stops_at_the_eight_megabyte_budget, a_forward_page_is_cut_before_a_row_that_exceeds_the_payload_budget}` (a 9.6 MiB history cut by the read bound; one 9 MiB row deferred by the payload bound with the cursor pointing at it); `database::reclaim_returns_freed_pages_and_honours_both_budgets` (step budget and `min_free_pages`); CJK/emoji/NUL payloads in `history_query::tests::setup`. |
| `error-path` | `import_paths::unreadable_sources_and_missing_retry_targets_are_reported_not_imported` (file where a directory is expected, directory named like a transcript, retry source gone), `a_legacy_file_never_overwrites_an_existing_sqlite_session`, `events_are_validated_for_identity_sequence_and_duplicates` (identity mismatch, negative sequence, conflicting wire id, conflicting identity), `a_second_import_skips_committed_sessions_and_finishes_the_rest` (SQLite `RAISE(ABORT)` trigger mid-import → rollback, then a clean retry); `database::open_and_reclaim_paths::a_foreign_or_unrecognized_database_is_refused_instead_of_adopted`, `a_non_busy_transaction_failure_is_returned_instead_of_retried`; `persistence::commit_checkpoint_paths::a_failed_compaction_commit_is_surfaced_and_poisons_later_checkpoints` (injected commit failure, then the “refusing checkpoint after persistence failure” boundary); `fork::cutoff_paths::a_checkpoint_that_cannot_be_remapped_is_dropped_from_the_fork`; `manager::fork_request_validation`; `history_query` request validation. |
| `concurrency` | `database::open_and_reclaim_paths::a_locked_writer_is_retried_on_the_same_connection_until_it_is_free` — two real connections, `BEGIN IMMEDIATE` held by a second connection, worker busy-timeout set to 0 so the 20/40/80 ms retry ladder is what recovers; asserts the write landed. Plus the pre-existing `session::database::tests::worker_failure_is_reported_to_current_and_future_callers`, `sqlite_storage::concurrent_clones_share_one_ordered_writer` (integration) and `persistence::tests::writer_drop_during_execute_stops_worker` (worker dropped mid-command; now asserts the manager can still persist and reload). |
| `property` | Round-trip / invariant properties rather than examples: `history_index::forward_page_budget::the_projected_page_is_never_larger_than_the_bytes_that_were_read` (measured over a mixed page — text, CJK, a 64 KiB message, tool result with attachments, run markers, `session_info`: Σ projected ≤ Σ stored+overlay); `sqlite_store::journal_selection_paths::refreshing_a_session_info_keeps_one_record_with_the_latest_content` (append+replace keep exactly one row with the last snapshot); `records::canonical_paths::an_explicitly_empty_block_array_survives_a_storage_round_trip` (empty ≠ absent); `import_paths::entry_shapes…` (duplicate lines are idempotent) and `a_second_import_skips_committed_sessions_and_finishes_the_rest` (re-running an interrupted import converges); `persistence::commit_checkpoint_paths::commit_checkpoint_appends_its_entry_to_the_journal` (checkpoint survives close + reload); `fork::cutoff_paths::a_settled_turn_resolves_through_its_run_markers`. |
| `platform-cfg` | One platform-gated item, and it is a *test*: `agent/src/session/manager.rs:1661` `#[cfg(unix)] #[test] fn load_rejects_session_with_only_blank_lines`. On this Windows host it is neither compiled nor run — registered as `platform-unmeasured` in §6, with the Linux/macOS CI runners as the authoritative platform. No *production* item in this subtree is gated: storage goes through `rusqlite` (bundled SQLite) and `std::fs`, and the platform-specific sandbox code lives in `agent/src/sandbox/` (another group). |
| `serialization` | `import_paths` (whole-file JSONL → SQLite: legacy schema, retired compaction schema, legacy timestamp shapes, duplicate/conflicting identities, `meta` shape validation), `records::canonical_paths` (legacy `tool_calls`, `"Error:"`-prefixed tool text, explicit empty block array), `history_index::shape_reduces_legacy_and_block_tool_calls_to_pairing_ids` (legacy calls and call blocks both reduce to pairing ids without duplicating arguments), `projection::thinking_projection` (legacy `thinking` field → `Reasoning` block, exactly once), `display::projection_guards` (authoritative session_info replaces stale ones), `fork::cutoff_paths::a_checkpoint_that_cannot_be_remapped_is_dropped_from_the_fork` (checkpoint content rewritten to the child's ids). |

## 5. Weak tests found and fixed (`weak-tests-fixed`)

A scan of every `#[test]`/`#[tokio::test]` in `agent/src/session/**` +
`agent/src/logfile.rs` for bodies with no assertion found **three** offenders, all
in `agent/src/session/persistence.rs`; all three were fixed rather than justified:

| test | before | after |
|---|---|---|
| `persistence::tests::writer_drop_during_execute_stops_worker` | dropped the writer mid-command and asserted nothing (a hang would have been the only failure signal) | now asserts the manager can still `save` and that the reloaded session has the same entry count |
| `persistence::tests::save_retry_succeeds_after_transient_failures` | called `save_with_retry` and dropped the result | asserts both injected failures were consumed (`fail_saves_remaining == 0`) and that the retry persisted the whole session |
| `persistence::tests::commit_without_prior_session_info_skips_merge` | committed a run marker and asserted nothing | asserts the terminal marker reached the journal and that no `session_info` row was invented for a session that never had one |

A fourth, self-inflicted case was caught by measuring rather than by scanning and is
described in §7.2: a new test that *passed* while exercising a different bound than
its name and comment claimed. It was rewritten so that its assertions encode what it
actually proves.

The same scan reports **0** remaining assertion-free tests and **0** `#[ignore]`
markers in this subtree. The repo-wide audit document
(`docs/testing/weak-test-audit.md`) is not part of this subtree's write set, so this
section is this subtree's contribution to it.

## 6. Waivers — every file that still has uncovered lines

Categories and reasons are per line group. Paths are repo-relative (full paths, not
filenames). Line numbers come from the LCOV export described in §1; counts are the
summary's `Missed Lines`.

| file | lines | category | reason / proof |
|---|---|---|---|
| `agent/src/session/checkpoint.rs` | 91 | unreachable-by-construction | `checkpoint_is_valid`'s lookup of the checkpoint's own `entry_id` cannot fail: `checkpoint_to_entry` writes `id = checkpoint.entry_id`, and the only caller (`latest_context_checkpoint`) derives each candidate from an entry of the very slice it searches. Kept as a defensive guard; the type system cannot express “this checkpoint came from these entries”. |
| `agent/src/session/compaction_ops.rs` | 80, 112 | attribution-artifact | Closing braces of the cached-claim block and of the `if let Some(entry)` block. `session::compaction_ops_tests::content_and_parameters_invalidate_but_failed_attempt_is_replayed` asserts `CompactionClaim::Cached { .. }`, which requires the block on 68–81 to have run (its `return Ok(Cached…)` is covered). |
| `agent/src/session/database.rs` | 34, 282, 299, 309, 556, 609, 622, 711, 737, 848 | attribution-artifact | Braces / `?` continuations of statements a named test executes: `database::tests::committed_data_survives_reopen_and_failed_transaction_rolls_back` (34), `open_and_reclaim_paths::a_foreign_or_unrecognized_database_is_refused_instead_of_adopted` (282, 299), `database::tests::pre_release_layout_is_rejected_instead_of_upgraded` (309, 711), `database::tests::opening_repairs_fork_session_creation_time_from_the_operation_boundary` (609, 622), `database::tests::entry_identity_is_unique_within_a_session_not_globally` (737), `a_non_busy_transaction_failure_is_returned_instead_of_retried` (848). |
| `agent/src/session/database.rs` | 128-130, 164-166, 268-270, 273 | attribution-artifact | Field expressions inside `tracing::info!/debug!` on the same statements as the log call sites, which are covered (`ensure_incremental_auto_vacuum`'s conversion succeeds on every fresh database; `reclaim`'s “reclaimed N pages” fires in `open_and_reclaim_paths::reclaim_returns_freed_pages_and_honours_both_budgets`). No subscriber is installed in the measured test binary, so the field expressions are never evaluated. |
| `agent/src/session/database.rs` | 96-98, 120, 124, 170, 172, 194, 261 | unreachable-in-this-environment | Arms that need a real I/O fault this environment cannot produce deterministically: a failing `pragma_query_value` on a healthy connection (96-98), a failing `VACUUM`/`PRAGMA auto_vacuum` conversion during `ensure_incremental_auto_vacuum` (120-124) and inside `compact_if_bloated` (170-172, whose `>64 MiB` bloated precondition was not built here), losing the worker-start handshake between `spawn` and `recv` (194), and the “last page is in use” break in the reclaim loop (261 — measured: a 501-page freelist produced by this fixture was always fully truncatable, so the loop's progress guard never fired). Declared gap, not a proof of unreachability. |
| `agent/src/session/fork.rs` | 118, 142, 194, 205, 384 | attribution-artifact | Region ends inside `retain_mut`/`remap`/`resolve_cutoff`, whose statements are covered by `fork::cutoff_paths::a_checkpoint_that_cannot_be_remapped_is_dropped_from_the_fork` and `a_settled_turn_resolves_through_its_run_markers`. |
| `agent/src/session/fork.rs` | 202 | attribution-artifact | The second `return false` inside the checkpoint-summary remap (evidence references outside the copied range). The surrounding `if let Some(summary)` branch is proven to run by the named test above (its failing-summary sibling on 195-199 fires for the `"not-blocks"` case); this specific failure mode was not constructed in this segment. Declared gap. |
| `agent/src/session/fork.rs` | 485, 500 | unreachable-by-construction | Test-side `panic!` arms (`checkpoint expected`, `text expected`) that exist to fail the test; while the test passes they cannot execute. |
| `agent/src/session/history_index.rs` | 65 | unreachable-by-construction | That `match` arm is fed only blocks whose `"type"` is `tool_result` (filter on line 57), and a `tool_result` tag can only deserialize into `ContentBlock::ToolResult`; any other variant is unreachable there. |
| `agent/src/session/history_index.rs` | 101, 108, 139, 144, 165 | attribution-artifact | `?`/region ends of `insert_shape`, `reconcile` and the materialiser, all executed by `history_index::tests::paged_projection_matches_full_history_and_reconciles_appends`, `cold_page_reads_only_selected_bodies` and `forward_page_budget::the_projected_page_is_never_larger_than_the_bytes_that_were_read`. |
| `agent/src/session/history_query.rs` | 37, 131, 151 | attribution-artifact | Continuation lines of the `require_session` query, the total-bytes query and the `substr` read; executed by `history_query::tests::out_of_range_search_and_read_requests_are_rejected_before_touching_storage` (whose “offset is beyond the entry's N readable bytes” assertion requires 128-131 to run) and by the existing snippet test. |
| `agent/src/session/legacy_import.rs` | 20, 284 | attribution-artifact | `matches!(…)` head of `source_unreadable` and the `?` of the `storage_meta` insert. `legacy_import::import_paths::unreadable_sources_and_missing_retry_targets_are_reported_not_imported` asserts the session was skipped with kind `source_unreadable` — impossible unless `source_unreadable` returned true — and `a_second_import_skips_committed_sessions_and_finishes_the_rest` asserts the completed re-import, which requires the `legacy_complete` insert. |
| `agent/src/session/legacy_import.rs` | 42, 233 | unreachable-in-this-environment | Catch-all arms for a `read`/`read_dir` error outside the five kinds `source_unreadable` accepts; every deterministic fixture (missing file, removed file, directory where a file is expected, file where a directory is expected) maps onto one of those kinds, so the catch-all is not reachable from a fixture this environment can build. Declared gap. |
| `agent/src/session/manager.rs` | 1661 | platform-unmeasured | `#[cfg(unix)] #[test] fn load_rejects_session_with_only_blank_lines` — a test compiled out on this Windows host, so it neither runs nor appears in the report. Authoritative platform: the Linux/macOS CI runners, where it runs as part of `cargo test -p future-agent --lib`. |
| `agent/src/session/manager.rs` | 96, 410, 431 | attribution-artifact | Region ends of `storage()`'s initialisation branch and of `load_metadata`/`restore_session_times`'s `query_row` calls; executed by every test that loads a session (`display_cache_paths::a_cache_hit_is_promoted_and_the_oldest_entry_is_evicted` calls `session_revision`, which goes through the same storage path). |
| `agent/src/session/persistence.rs` | 790 | attribution-artifact | The early `return` of `merge_latest_session_info` when the session has no `session_info`; `persistence::tests::commit_without_prior_session_info_skips_merge` asserts the terminal committed and no `session_info` was invented, which is only possible if that branch was taken. |
| `agent/src/session/records.rs` | 15 | unreachable-by-construction | `canonical_entry`'s non-object arm: it is only reached after `value["type"]` matched an entry-type string, and `serde_json`'s `Index` yields `Null` for every non-object value, so no non-object can both match the entry types and take that arm. |
| `agent/src/session/records.rs` | 129, 134, 144, 211, 283 | attribution-artifact | `?`/region ends of the `created_at_ms` update, the `session_info` metadata update, the `model_change` update and the explicit-empty-array update, plus one pre-existing test continuation. Executed by `records::canonical_paths::an_explicitly_empty_block_array_survives_a_storage_round_trip` (211 — the assertion requires the `content_json='[]'` update to have run), `records::tests::relational_blocks_preserve_order_and_provider_extensions` and `sqlite_storage::current_metadata_and_per_run_usage_survive_rewrite_and_reopen`. |
| `agent/src/session/repair.rs` | 90 | attribution-artifact | The `tracing::warn!` field expression; the statement itself runs in `repair::dedupe_and_orphan_paths::a_real_result_replaces_the_placeholder_for_its_call` (its `true` return requires the placeholder to have been dropped and the log to have been emitted). |
| `agent/src/session/sqlite_store.rs` | 93, 337, 416, 451, 515, 684, 802 | attribution-artifact | `?`/`}` continuations: the pricing-event query (93) and `has_events` (337) are executed by `sqlite_store::journal_selection_paths::only_pricing_events_are_read_back_and_identical_appends_are_no_ops`; the entry-position insert (416), the synthesised `run_id` (451) and the metadata-position delete (515) by that module's metadata test and by `sqlite_storage.rs`; 684 and 802 are pre-existing test-code continuations. |
| `agent/src/session/sqlite_store.rs` | 139 | unreachable-in-this-environment | The `tracing::warn!` arm taken only when `Database::reclaim` fails (a SQLite/IO error inside the worker); no deterministic fixture produces it. Declared gap. |
| `agent/src/session/summary.rs` | 55, 99 | attribution-artifact | A closing brace inside `summary_from_session` and the `json!`/`from_value` tail of `list_all`'s synthetic tail entry (97-99). Both are executed by the existing `summary` tests and by the `manager` list tests (which is why 97-98 are covered while the macro tail is not). |
| `agent/src/logfile.rs` | 2 missed lines | attribution-artifact | Neither `llvm-cov show` nor the LCOV export marks any zero-count line in this file — the two lines exist only in the summary's line metric (macro/span attribution). Every function in the file is exercised through the session tests that write and re-read JSONL sessions (`persistence::commit_checkpoint_paths`, `save_with_retry`, the reopen tests). |

## 7. Findings, bugs and known gaps

1. **Measurement fragility (affects the whole goal).** On this shared checkout, a
   single failing test *anywhere* in `future-agent` — or another worker's in-flight
   compile error — makes `cargo llvm-cov --no-report` lose the lib test binary's
   profile data; the resulting report understates the subtree by 70+ percentage
   points and would also *hide* real gaps behind inflated waiver counts. This
   segment needed three waits (up to ~25 minutes) for other workers' subtrees to
   compile before a trustworthy measurement was possible. The workaround used here
   (a precise `--skip` list naming only other groups' red tests, then a green run)
   is recorded verbatim in §1 so the number can be reproduced.
2. **A passing test is not a test of what its name says.** The first version of my
   payload-bound test (`an_oversized_row_ends_the_forward_page_at_itself`) passed
   with assertions that *looked* like proof of the payload bound, but measuring
   showed `materialize`'s payload guard still uncovered: the row I made oversized
   was a `session_info`, and the store keeps one *current* metadata record written
   at the **original metadata position**, so the huge row landed first — where the
   guard is deliberately skipped (`!entries.is_empty()`). The old assertions had
   been satisfied by `read_page`'s bound instead. Rewriting it with the oversized
   row *later* in the page both covers the guard and shows real behaviour: the page
   stops before it, `nextOffset` points at it, and the next page returns it. The
   lesson for the rest of the goal: verify *which* line a new test covers, not just
   that the suite is green.
3. **No production bugs found** in this subtree. Two behaviours are worth keeping
   an eye on, both now covered: a repeated legacy import skips already-committed
   sessions only while `legacy_complete` is unset (an import that fails midway
   re-walks the directory — `a_second_import_skips_committed_sessions_and_finishes_the_rest`),
   and a `compaction` checkpoint whose summary cannot be remapped is silently
   dropped from a fork rather than failing it
   (`a_checkpoint_that_cannot_be_remapped_is_dropped_from_the_fork`).
4. **Known gaps** (declared above, not disguised as waivers): `fork.rs:202`,
   `database.rs:194/261`, `legacy_import.rs:42/233`, `sqlite_store.rs:139`, the
   four `tracing`-field groups, and the Windows-blind `#[cfg(unix)]` test at
   `manager.rs:1661` (a `platform-unmeasured` line whose authority is the
   Linux/macOS CI runners). Next useful check for a later segment: install a
   thread-local `tracing` subscriber in one test module (the crate already depends
   on `tracing-subscriber`) and re-measure — that should retire the
   `tracing`-field attribution rows; and build the “evidence references outside
   the copied range” fixture to retire `fork.rs:202`.
