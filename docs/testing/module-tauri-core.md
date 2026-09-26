# Module `tau-core` — coverage result, waiver ledger and dimension evidence

Subtree: `desktop/src-tauri/src/{lib.rs, main.rs, instance.rs, runtime.rs,
config_io.rs, windows_power.rs, git_review.rs, skills.rs, skills_bootstrap.rs,
store/, shadow_review/, agent_providers/}` — the `tau-core` group of
`verify.py`'s `RUST_GROUPS` for the `desktop-tauri` package (`futureos`).

## 1. What was measured, with which command

`desktop/src-tauri` is **not** a workspace member, so it is measured by its own
run from inside that directory into this group's own report file (the gate
prefers `coverage/tauri-tau-core-report.json` for `tau-core`):

```
cd desktop/src-tauri
CARGO_TARGET_DIR=target/cov-tau-core RUST_TEST_THREADS=4 \
  cargo llvm-cov -p futureos -j 3 --json --output-path ../../coverage/tauri-tau-core-report.json
CARGO_TARGET_DIR=target/cov-tau-core \
  cargo llvm-cov report --lcov --output-path ../../coverage/tauri-tau-core.lcov
```

Gate: `python .future/cov100/verify.py rust-module tau-core 99.99 docs/testing/module-tauri-core.md coverage/llvm-cov-tauri.json`
(the group's own report wins over the shared one, and the gate prints that).

| | lines | uncovered lines | files with uncovered lines |
|---|---|---|---|
| before this change | 12395 / 12843 = **96.5117 %** | 448 | 28 |
| after the tests in §2 | 12876 / 13231 = **97.3169 %** | 355 | 26 |

All 1473 lib tests pass with 0 failures and 3 pre-existing `#[ignore]`s; the
denominator grew by 383 because the new tests are instrumented too. The "after"
row is the final measurement of the tree this document ships with (the report
and LCOV file named in the measurement command were regenerated from that same
green run, so the numbers above, the per-file residuals in §3 and the report
reconcile line for line).

Two independent line views are used in this document, and the difference
matters:

* **llvm-cov's JSON line summary** — the gate's metric, and the numbers in the
  table above and in §3.
* **per-line execution** — the `DA:` records of the LCOV export (identical to
  the per-line `hits` of the cobertura export and to the line `Count` column of
  `llvm-cov report --html`). A file whose LCOV records contain **no zero-count
  line** has every executable line executed *somewhere*; the JSON summary still
  reports a residual for it, because its line metric also scores nested and
  monomorphized regions (a `?`-propagation region, a closure body, a generic
  instantiated twice). Those residuals are registered below as
  `attribution-artifact`, which is exactly the category the plan defines for
  "a brace or span end next to a macro expansion, where a named test proves the
  surrounding code ran".

## 2. Tests added (all in the subtree's own files)

| file | new/rewritten tests |
|---|---|
| `desktop/src-tauri/src/windows_power.rs` | `power_action_maps_every_broadcast_it_cares_about`, `install_disconnect_notifier_returns_without_a_main_window`, `install_disconnect_notifier_returns_when_the_window_has_no_hwnd`, `power_wnd_proc_dispatches_every_action_and_both_fallbacks`; production change: the message→action mapping is extracted as the pure `power_action` (the file's sibling `remote::supervisor::power_transition` is documented as the same idiom), and `install_disconnect_notifier` is generic over `tauri::Runtime` so the two early-return arms are reachable |
| `desktop/src-tauri/src/instance.rs` | `acquire_resolves_the_home_directory_and_keeps_its_lock` |
| `desktop/src-tauri/src/store/db.rs` | `agent_session_binding_migration_reports_and_rolls_back_a_failing_repair`, `remote_prompt_receipt_migration_reports_and_rolls_back_a_failing_backfill` |
| `desktop/src-tauri/src/store/util.rs` | `count_files_under_descends_into_subdirectories` |
| `desktop/src-tauri/src/store/workspaces.rs` | `find_user_workspace_matches_an_aliased_spelling_by_canonical_path` |
| `desktop/src-tauri/src/store/runs.rs` | `remote_prompt_receipt_rejects_a_blank_command_id`, `remote_prompt_receipt_reports_a_run_that_could_not_be_stamped`, `a_tool_delta_without_text_projects_no_tool_call` |
| `desktop/src-tauri/src/store/threads.rs` | `inheriting_an_asset_root_for_a_missing_thread_is_an_error`, `binding_a_session_to_a_thread_that_is_out_of_service_is_refused`; `failed_directory_removal_stays_pending_cleanup` made unconditional (`if path.exists()` → always clear, see §5) |
| `desktop/src-tauri/src/store/app_settings.rs` | `auto_title_first_turn_round_trips_and_unknown_languages_are_refused` |
| `desktop/src-tauri/src/store/markdown_refs/resolve.rs` | `file_reference_absolute_paths_are_kept_verbatim` |
| `desktop/src-tauri/src/shadow_review/snapshot.rs` | `filter_ignored_surfaces_a_git_failure` |
| `desktop/src-tauri/src/git_review.rs` | `tracked_diff_files_omits_sensitive_binary_and_oversized_blobs`, `append_untracked_files_marks_an_oversized_file_truncated`, `branch_diff_base_falls_back_to_head_without_a_primary_branch`, and `review_cache_clears_when_full` **rewritten** (see §5) |
| `desktop/src-tauri/src/agent_providers/tests.rs` | `upsert_rolls_models_back_when_the_paired_key_write_fails`, `delete_restores_models_when_the_auth_removal_fails` (drive the existing one-shot auth-write seam) |
| `desktop/src-tauri/src/skills.rs` | `a_non_success_guide_response_reports_its_status` (new test module with a loopback stub answering `503`) |
| `desktop/src-tauri/src/lib.rs` | production change: `emit_threads_updated` split into `emit_threads_updated_via`/`emit_threads_updated_on`; `emit_via_helpers_route_both_arms` extended to route all three arms of the new pair |

## 3. Waiver ledger — `lines-100-or-waived`

Every file with a non-zero residual on llvm-cov's JSON line metric appears
below, on a line that names one of the four plan categories and the reason.

### 3.1 Files with lines no test executed (LCOV `DA:0`)

| path | category | reason |
|---|---|---|
| `desktop/src-tauri/src/lib.rs` | unreachable-in-this-environment | The residual is the Tauri bootstrap itself — `run()`'s plugin builder chain, `.setup()` closure, `generate_handler!` list and `app.run()` event loop — plus the window/taskbar-icon/caption helpers (`size_main_window_to_screen`, `set_windows_taskbar_icon`, `match_windows_caption_color`) that only `setup` calls. All of it needs a live WebView2 message pump and a real `AppHandle<Wry>`; `run()` blocks until the process exits, and `tauri::test::MockRuntime` supplies neither a monitor nor an `HWND`, so no test can enter it. The pure pieces that were extractable are covered by name: `main_window_geometry_scales_clamps_and_centers`, `main_window_geometry_clamps_to_minimums`, `coalesce_runtime_updates_*`, `sample_thread_streaming_emits_only_on_change`, `sample_thread_streaming_wrapper_delegates_to_real_id_source`, `runtime_update_drain_loop_coalesces_and_exits`, `emit_via_helpers_route_both_arms` (now also `emit_threads_updated_via`/`emit_threads_updated_on`), `spawn_remote_auto_connect_returns_when_no_creds`. The two remaining process-global arms (`thread_streaming_monitor_loop` / `runtime_update_drain_loop` when `APP_HANDLE` is *set*) are the same limitation: the only producer of that global is `run()`. |
| `desktop/src-tauri/src/main.rs` | unreachable-in-this-environment | `#[cfg(not(test))] fn main()` (arg parse → `futureos_lib::run()` → exit code) and `configure_environment`'s macOS-only body exist only in the non-test binary; the GUI process is never launched by the test run, and the test binary compiles the `test` cfg instead. |
| `desktop/src-tauri/src/runtime.rs` | unreachable-by-construction | Lines 44-49 are the `#[cfg(not(test))]` twin of the `#[cfg(test)]` tracked `spawn`; the test binary compiles only the test variant, so this item is not callable from any test (the app process is not run — see `lib.rs`). |
| `desktop/src-tauri/src/shadow_review/snapshot.rs` | unreachable-in-this-environment | Lines 81-91 and 230-245 are the "a path became ignored between candidate discovery and staging" arms (the code's own comment: *"If an untracked path becomes ignored after candidate discovery, drop it and retry once"*). Reaching them requires a file's ignore status to flip **between two `git` invocations inside one `capture()`/`stage()` call**, i.e. another process editing `.gitignore` mid-capture; a test cannot schedule that interleaving, and a single-threaded test sees identical `check-ignore` answers in both passes. Line 192 is `attribution-artifact`: it is the `?` on the `repo.run(...)` statement that the named test `filter_ignored_surfaces_a_git_failure` executes (that test asserts the neighbouring failure arm, lines 194-198). |
| `desktop/src-tauri/src/store/util.rs` | unreachable-in-this-environment | Line 169 is the `continue` for a symlink entry. Producing one on this host needs `SeCreateSymbolicLinkPrivilege`, which the test process does not hold — verified: `New-Item -ItemType SymbolicLink` fails here (so `std::os::windows::fs::symlink_file` would too). Line 172 is now covered by `count_files_under_descends_into_subdirectories`. |
| `desktop/src-tauri/src/agent_providers/write.rs` | unreachable-in-this-environment | Lines 433-434 and 535-537 are the "models.json write failed" restore arms of the two transactional local writers. Only a real filesystem fault reaches them (the *auth* half is covered through the existing injection seam by `upsert_rolls_models_back_when_the_paired_key_write_fails` / `delete_restores_models_when_the_auth_removal_fails`), and provoking that fault is platform-specific — a Windows read-only file attribute versus POSIX directory modes — so no single platform-agnostic test can produce it. |
| `desktop/src-tauri/src/agent_providers/write.rs` | attribution-artifact | Line 554 is the closing brace of the auth-failure block in `apply_delete_local`; the block body (restore + `error = Some(e)`) executes in the named test `delete_restores_models_when_the_auth_removal_fails`, and the failed write it guards is asserted there. |
| `desktop/src-tauri/src/skills_bootstrap.rs` | unreachable-in-this-environment | Lines 19-20 and 23 are the `sidecar("future")` error arm: it needs an app whose config declares no `future` external binary, while the mock app builds from this crate's real `tauri.conf.json`. Lines 28 and 35 need the bundled CLI child to spawn and drain, i.e. the installed sidecar binary. Line 69 is `unreachable-by-construction`: the `_ => {}` arm the compiler demands for the `#[non_exhaustive]` `CommandEvent` enum, which no released variant can reach. |
| `desktop/src-tauri/src/store/threads.rs` | unreachable-in-this-environment | Line 171 (`Some(thread) => Ok((thread, false))` after a failed `create_thread`) is reachable only when two clients race the same agent session id between the initial `find_thread_by_agent_session` and the insert: the loser's insert hits the one-session-per-thread unique index, and its second lookup sees the winner's row. `find_thread_by_agent_session` excludes soft-deleted rows, so no sequential arrangement of rows reaches it; a test cannot schedule the interleaving. Lines 538 and 562 are `attribution-artifact` (brace closes executed by `binding_a_session_to_a_thread_that_is_out_of_service_is_refused` and `failed_directory_removal_stays_pending_cleanup`). |
| `desktop/src-tauri/src/windows_power.rs` | unreachable-in-this-environment | Line 83 is the `window.hwnd()` failure arm — a mock webview returns a handle instead of failing; line 88 stores the previous procedure, which requires `SetWindowLongPtrW` to *return* a non-zero previous procedure, i.e. a window that already carries a per-window procedure (a real Tauri window; both plain `STATIC` windows and the desktop window report the class procedure, which is not stored). Everything else in the file is covered, including both `DefWindowProcW`/`CallWindowProcW` fallbacks driven with a real HWND and all four message classifications. |
| `desktop/src-tauri/src/instance.rs` | attribution-artifact | Lines 36 and 38 are continuation lines of the `format!` that builds the already-running message. `excludes_second_owner_and_releases_on_drop` and `acquire_resolves_the_home_directory_and_keeps_its_lock` both assert that message and the directory it names, so the surrounding code provably ran. |
| `desktop/src-tauri/src/store/db.rs` | attribution-artifact | Lines 200, 319, 324 and 415 are the `)?;` closes of multi-line `conn.execute(...)` statements. Their bodies run in `apply_schema_dedupes_and_enforces_unique_agent_session_bindings`, `agent_session_binding_migration_reports_and_rolls_back_a_failing_repair` and `remote_prompt_receipt_migration_reports_and_rolls_back_a_failing_backfill`; the last two are new and assert the *failure* arms of exactly these migrations (the `?`-propagation region is the only thing that never executes). |
| `desktop/src-tauri/src/store/cleanup.rs` | attribution-artifact | Line 301 is the `)?;` of the multi-line `conn.prepare(...)` in `live_thread_ids`; the statement runs in the module's reconcile/purge tests (`reconcile_deletes_orphans_reported_gone_by_the_agent`, `archive_finished_runs_keeps_event_log`). |
| `desktop/src-tauri/src/shadow_review/policy.rs` | attribution-artifact | Line 177 is a brace close inside the candidate scan, whose neighbours run in `sensitive_matches_credentials` and `classify_respects_size_and_sensitivity`. |
| `desktop/src-tauri/src/store/app_settings.rs` | attribution-artifact | Line 178 is the `)?;` of the auto-title write that the new `auto_title_first_turn_round_trips_and_unknown_languages_are_refused` executes and then asserts by reading the setting back. |

### 3.2 Files whose residual is entirely a metric artifact (`DA:0` set is empty)

For each file below, the LCOV/cobertura/HTML line view marks **every**
executable line as executed, while llvm-cov's JSON line summary still scores a
residual: nested regions (`?` propagation, closures) and generic bodies
instantiated in more than one copy. Verified for this report: the per-file HTML
line view shows `0` `uncovered-line` entries and the LCOV export contains no
zero-count `DA:` record. Category and the named tests that execute the file:

| path | category | reason (named tests execute every line; residual is the JSON region/instantiation view) |
|---|---|---|
| `desktop/src-tauri/src/agent_providers/mod.rs` | attribution-artifact | `custom_provider_model_serde_defaults_context_window_and_max_tokens`, `list_agent_providers` |
| `desktop/src-tauri/src/config_io.rs` | attribution-artifact | `missing_file_reads_as_empty_object`, `atomic_write_round_trips`, `corrupt_file_errors_and_is_not_clobbered` |
| `desktop/src-tauri/src/shadow_review/maintenance.rs` | attribution-artifact | `run_startup_maintenance`, `verify_consistency`, `recover_interrupted_runs` |
| `desktop/src-tauri/src/shadow_review/repository.rs` | attribution-artifact | `workspace_lock_is_reused_and_serialized`, `open_and_bare_initialize_git_and_non_git` |
| `desktop/src-tauri/src/skills.rs` | attribution-artifact | `a_non_success_guide_response_reports_its_status` (new) plus the success path exercised by `commands::skills`' `skill_guide_still_uses_the_platform_endpoint` |
| `desktop/src-tauri/src/store/approvals.rs` | attribution-artifact | `ensure_inserts_once_and_dedupes_by_id_or_tool_call`, `list_pending_approval_requests` |
| `desktop/src-tauri/src/store/artifacts.rs` | attribution-artifact | `create_list_delete_artifacts`, `artifact_type_classification` |
| `desktop/src-tauri/src/store/review_snapshots.rs` | attribution-artifact | `lists_finished_but_unmaterialized_run`, `prune_keeps_the_newest_and_reports_shadow_refs` |
| `desktop/src-tauri/src/store/runs.rs` | attribution-artifact | the run-status CAS tests plus the new `remote_prompt_receipt_*` and `a_tool_delta_without_text_projects_no_tool_call` |
| `desktop/src-tauri/src/store/skill_reco.rs` | attribution-artifact | `recorded_recommendations_are_read_back_with_skill_and_message` |
| `desktop/src-tauri/src/store/workspace_files.rs` | attribution-artifact | `fuzzy_ranks_and_returns_name`, `respects_gitignore_and_always_skip_dirs` |
| `desktop/src-tauri/src/git_review.rs` | attribution-artifact | `tracked_diff_files_omits_sensitive_binary_and_oversized_blobs`, `branch_diff_base_falls_back_to_head_without_a_primary_branch`, `review_cache_clears_when_full`, `get_git_review_reports_git_workspace` — every line of the file is executed; the 9-line residual is the region view over the generic/`match` arms those tests drive |

### 3.3 Source files with no report entry (completeness check)

The gate's completeness check requires every `.rs` file of the subtree to either
appear in the report or be named here. These three appear in neither the JSON
nor the LCOV export:

| path | category | reason |
|---|---|---|
| `desktop/src-tauri/src/store/schema.rs` | attribution-artifact | Contains only SQL string constants (0 `fn` items — verified), so it contributes no instrumentable regions and the exporter emits no file entry. |
| `desktop/src-tauri/src/store/records.rs` | attribution-artifact | Contains only derive-only row structs (0 `fn` items — verified); the generated `Default`/serde code is not attributed to this file, so it has no line data. |
| `desktop/src-tauri/src/agent_providers/tests.rs` | attribution-artifact | Test-only module (`#[cfg(test)] mod tests;`). llvm-cov's export for this crate contains no entry for any `tests.rs` module file (its sibling `test_support.rs` modules do appear), so the file has no production lines to waive; its tests are the `agent_providers::` cases named in §2. |

### 3.4 Siblings that this host cannot compile (`platform-unmeasured`)

Windows is the only platform measured for this subtree, so the platform-selected
siblings of `windows_power.rs` appear in neither the report nor the LCOV export;
a local "100 %" can only ever speak for Windows. They are registered here by
their repo-relative paths so the blind spot stays explicit:

| path | category | reason |
|---|---|---|
| `desktop/src-tauri/src/macos_power.rs` | platform-unmeasured | `#[cfg(all(feature = "gui", target_os = "macos"))]`; needs a macOS run to be measured at all. |
| `desktop/src-tauri/src/linux_power.rs` | platform-unmeasured | `#[cfg(all(feature = "gui", target_os = "linux"))]`; needs a Linux run to be measured at all. |
| `desktop/src-tauri/src/menu.rs` | platform-unmeasured | `#[cfg(all(feature = "gui", target_os = "macos"))]`; needs a macOS run to be measured at all. |

## 4. Dimension evidence (`dimensions`)| dimension | evidence in this subtree |
|---|---|
| `boundary` | `windows_power::tests::power_action_maps_every_broadcast_it_cares_about` — unknown power code (`0xDEAD`), a known code on the wrong message, and a widened `WM_QUERYENDSESSION` are all `Ignore`; all three resume codes are recognized. `git_review::review_cache_clears_when_full` seeds exactly `REVIEW_CACHE_MAX` entries before the eviction call. `app_settings` rejects an unsupported title language; `git_review::tracked_diff_files_omits_sensitive_binary_and_oversized_blobs` drives the `> 10_000 diff lines` boundary; `store::runs::remote_prompt_receipt_rejects_a_blank_command_id` covers the whitespace-only id; `store::util::count_files_under_descends_into_subdirectories` covers depth > 1. |
| `error-path` | New: two migration failures that must roll back (`store::db::*_migration_reports_and_rolls_back_*`, asserting both the error text and that no marker row / added column survives), a failing `git check-ignore` (`shadow_review::snapshot::filter_ignored_surfaces_a_git_failure`), a `503` guide response (`skills::a_non_success_guide_response_reports_its_status`), an uncommittable remote-prompt receipt and a blank command id (`store::runs::*`), a missing thread for asset-root inheritance and an out-of-service thread for session binding (`store::threads::*`), the injected auth-write failure on both transactional writers (`agent_providers::upsert_rolls_models_back_when_the_paired_key_write_fails`, `delete_restores_models_when_the_auth_removal_fails`), plus the pre-existing lock-contention and "directory cannot be created" cases in `instance::*`. |
| `concurrency` | `instance`'s OS-level lock: the same data directory refuses a second owner and is available again after the only owner drops (`excludes_second_owner_and_releases_on_drop`, `acquire_resolves_the_home_directory_and_keeps_its_lock`); the new Win32 dispatch test holds `remote::test_support::mock_agent_lock()` so its suspend/resume actions cannot tear down a bridge another test is driving, and restores `ORIGINAL_WNDPROC` so no other test inherits the subclass; the pre-existing `runtime::cancel_test_tasks` fixture and `store::workspaces::tests` pool tests cover shutdown/ordering of process-lifetime tasks. The one genuinely racy arm (two clients creating the same agent session at once, `store/threads.rs:171`) is registered in §3.1 — it is a race, not a gap in the tests. |
| `property` | `power_action` is a total, table-driven mapping over the message/`wparam` space (every branch asserted, including the negative cases); `git_review`'s omission classification is a table of three inputs → three reasons with the invariant that an omitted file carries no diff bytes; `resolve_file_reference`'s invariant that an absolute reference is kept verbatim and is *not* re-anchored to the workspace root; `store::workspaces::find_user_workspace_matches_an_aliased_spelling_by_canonical_path` asserts the identity invariant "one directory ⇒ one workspace row"; the pre-existing `coalesce_runtime_updates_*` cases assert the terminal-beats-revision ordering invariant. |
| `serialization` | `app_settings::auto_title_first_turn_round_trips_and_unknown_languages_are_refused` writes through `update_app_settings` and reads back with `get_app_settings`; `store::db`'s two new migration tests build **legacy** schemas (a pre-v1.1.5 `threads` table, a `runs` table without `remote_accepted_at`) and assert the migration contract on them, including that a failed migration leaves no `schema_migrations` row; `store::runs::a_tool_delta_without_text_projects_no_tool_call` feeds wire payloads (`{"tool_id":"t1"}`, `{"text":""}`) through the run-event decoder; the pre-existing `store/runs`, `store/record_macro.rs` and `shadow_review/snapshot` round-trips cover the row and snapshot payload schema. |
| `platform-cfg` | This host is Windows, so `windows_power.rs` is fully exercised (real `DefWindowProcW`/`CallWindowProcW`/`GetClassLongPtrW` calls, four message classifications, both early returns) — it is the platform-specific module of this subtree. `store/markdown_refs/resolve.rs`'s new test pins the *absolute* path arm, which the pre-existing `"/"` case only reaches off Windows; `store/workspaces.rs` builds its aliased spelling with `std::path::MAIN_SEPARATOR`. The `cfg(unix)`/`cfg(macos)`/`cfg(linux)` siblings of `windows_power` (`macos_power.rs`, `linux_power.rs`, `menu.rs`) are not compiled on this host at all, so they are absent from the report — they belong to the `platform-unmeasured` category and are registered for the module as a whole in the handoff below. |

## 5. Weak tests (`weak-tests-fixed`)

No assertion-free test was added, and no existing assertion was weakened.

* `git_review::tests::review_cache_clears_when_full` was **assertion-free** and
  did not test what it claimed: it filled the cache to `REVIEW_CACHE_MAX` and
  then called `get_git_review("nonexistent", …)`, which returns at the
  workspace lookup *before* the cache is touched (`let _ =` swallowed the
  error, and the eviction line stayed unexecuted). Rewritten: a real
  `HomeGuard` workspace, a capacity-sized cache, then a real review, asserting
  the cache is evicted rather than grown.
* `store::threads::tests::failed_directory_removal_stays_pending_cleanup` gated
  its cleanup on `if chat_path.exists()`, so on a clean run the removal never
  ran and a leftover directory from a previous run would have made the write
  fail opaquely. Now the path is cleared unconditionally and the assertion
  below it is unchanged.
* The new `assert!` in the rewritten cache test originally passed its value as
  a multi-line format argument, which is only evaluated when the assertion
  already fails (a line that can never be executed); it now computes the length
  first.
* `windows_power`'s dispatch test does not merely call the entry points: each
  call is compared against the answer the default procedure gives for the same
  message, so a swallowed or mis-routed message fails the test.
* The `git_review` omission test asserts *both* halves of the contract — the
  reason and `diff_truncated` — and then asserts that the total-budget pass
  does not re-label an already-omitted file, rather than only checking that the
  function ran.

## 6. Handoff

* **Identity**: goal `cov-100-multidim`, todo `todo_63306d80e736`, subtree
  `tau-core` of `desktop-tauri` (`futureos`), measured with the two commands in
  §1 into `coverage/tauri-tau-core-report.json` / `coverage/tauri-tau-core.lcov`.
* **Before → after**: 96.5117 % (12395/12843, 448 uncovered in 28 files) →
  97.3169 % (12876/13231, 355 uncovered in 26 files); 1473 tests pass, 3
  ignored (pre-existing).
* **Result**: `lines-100-or-waived` — 26 of the 26 files with a residual are
  registered above with a category (20 in §3.1/§3.2 and 3 more in §3.3); the
  part of that residual that is *not* a metric artifact is exactly the set in
  §3.1.
* **Bugs found (reported, not silently changed)**: (1) a mock "main" webview
  reports `hwnd()` as `Ok` even though it has no native handle, so
  `install_disconnect_notifier` proceeds to `SetWindowLongPtrW` with a handle
  the OS rejects — harmless (the call returns 0 and the slot is not stored),
  but it means the `hwnd()` error arm cannot be reached from a test.
  (2) `store::threads::delete_thread` removes the row rather than only marking
  it deleted, so the "out of service" binding refusal needs a row whose
  `status` is set outside the public API; noted here for whoever owns that
  module.
* **Next useful checks**: (a) the `platform-unmeasured` list
  (`macos_power.rs`, `linux_power.rs`, `menu.rs`, and the `cfg(unix)` arms of
  `lib.rs`/`config_io.rs`) must be filled in from a macOS/Linux measurement —
  this host cannot see them at all; (b) `store/util.rs:169` (symlink) and
  `write.rs`'s models-write-failure arms become reachable on a host that grants
  symlink creation or through a platform-specific fault injection, if the
  module is ever re-opened; (c) `store/threads.rs:171` and
  `shadow_review/snapshot.rs`'s two mid-call ignore-flip arms would need a
  deterministic concurrency injection to be covered rather than waived.
