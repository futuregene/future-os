# Platform coverage — what a Windows measurement cannot see

Every figure in this goal's module docs is measured on **Windows x86_64**.
`cargo llvm-cov` only reports code the current platform compiled, so anything
gated to another OS is **neither 0% nor in the denominator — it is absent**.
Therefore a local "100%" means *100% of Windows-reachable code*, and nothing
about the rest. This file lists what is outside that claim and names the
platform that does measure it.

How to reproduce the underlying check:

```bash
python .future/cov100/verify.py blindspot <crate> [doc]   # files absent from the report
python .future/cov100/verify.py crate <crate> 99.99 <report>  # fails if a source file contributes no data and is not waived
```

## Files not measured by a Windows run

| crate | file | why | authoritative platform |
|---|---|---|---|
| `future-agent` | `agent/src/sandbox/linux/helper.rs` | whole file gated: Linux only | Linux (CI) |
| `future-agent` | `agent/src/sandbox/seatbelt.rs` | whole file gated: macOS only | macOS (CI + release) |
| `future-tui` | `tui/src/terminal_posix.rs` | module gated by cfg(unix) | Linux/macOS (CI) |
| `desktop-tauri` | `desktop/src-tauri/src/linux_power.rs` | module gated by cfg(all(feature = "gui", target_os = "linux")) | Linux (CI) |
| `desktop-tauri` | `desktop/src-tauri/src/macos_power.rs` | module gated by cfg(all(feature = "gui", target_os = "macos")) | macOS (CI + release) |
| `desktop-tauri` | `desktop/src-tauri/src/menu.rs` | module gated by cfg(all(feature = "gui", target_os = "macos")) | macOS (CI + release) |

## Consequences for the goal's claims

1. **"All code at 100%" is not a claim this environment can make.** The
   claim it can make is: *every Windows-reachable line is covered or waived
   with a category*. The rows above are the boundary of that claim.
2. **The authoritative measurement for the rows above is a Linux/macOS CI run**
   with the same tooling. Until that is run, those files have no measured
   figure at all — they are not "uncovered", they are unmeasured.
3. **A private `target-dir` per worker is necessary but not sufficient** for a
   trustworthy parallel measurement: the Windows write-restriction capability
   is machine-global, so a concurrent `cargo-llvm-cov` can hold it and make
   restricted-runner tests print an explicit skip (silently raising the
   uncovered count). This is why `measure-final.py` refuses to run while any
   other llvm-cov process is alive.
4. **`cargo llvm-cov --no-report` silently drops the lib test data** when the
   `cargo test` invocation is not fully green, which makes a report *understate*
   with no error. `cargo llvm-cov --lib` is a different metric again (it drops
   `tests/*.rs` and bin targets). The gate's completeness check detects the
   first; the plan forbids the second.

## Regeneration (required at closeout)

This file is **generated** by `.future/cov100/gen-platform-coverage.py` and must be
regenerated against the FINAL authoritative report, then checked with:

```bash
python .future/cov100/verify.py blindspot <crate> docs/testing/platform-coverage.md
```

Two distinct questions, deliberately kept apart:

* **Platform gating** (the table above): the file is gated to another OS, so a
  Windows run cannot see it at all. Derived from `#![cfg(...)]` headers and
  `#[cfg(...)] mod x;` declarations found anywhere in the crate — not just in
  `mod.rs`, which is how `tui/src/terminal_posix.rs` was initially missed.
  Files gated `any(target_os = ..., test)` ARE compiled under `cfg(test)` and are
  therefore measured here; they do not belong in this table.
* **Absence from one report** (`blindspot`): can equally be caused by a per-group,
  aborted, or stale run rather than by platform gating. That list is a diagnostic,
  not the platform boundary.

## Test bodies this host does not execute

A second, different blind spot from the table above. The table lists files that
are not *compiled* here. This section lists test **functions** that compile but
whose bodies never run on Windows, because their cfg excludes it:

* `#[cfg(unix)]` / `#[cfg(all(test, unix))]` — Windows can never satisfy it.
* `#[cfg(not(target_os = "windows"))]` — the negation explicitly excludes this host.

These are not visible in any coverage number: the file is measured, and the
gated test simply never executes, so its body neither adds nor subtracts from the
totals. They were surfaced by a worker that added three such assertions and
stated plainly that "the Linux CI job is the first place they actually run" —
which is the honest form, and the reason this section exists rather than being
folded into a percentage.

Counted on this host: **116 gated-away `#[test]` function(s) in 46 file(s)**, plus 125 further gated helper(s) in 35 file(s) that those tests (or production) would use.

| file | gated-away `#[test]` function(s) |
|---|---|
| agent/src/sandbox/mod.rs | `classifies_legacy_bash_probe_exit_codes`, `legacy_bash_probe_short_circuits_non_bash_shells`, `legacy_bash_probe_runs_real_bash_version_check`, `on_path_scans_the_process_path`, `legacy_bash_probe_times_out_and_kills_child`, `shell_invocation_unix_passes_command_through_to_the_resolved_shell` … (+8 more) |
| tui/src/skills_cli.rs | `spawn_runner_captures_streams_and_the_exit_code`, `spawn_runner_reports_a_spawn_failure`, `spawn_runner_reports_a_signal_death_as_minus_one`, `wait_for_kills_a_child_that_outlives_the_timeout`, `wait_for_drains_stdout_larger_than_the_pipe_buffer`, `wait_for_drains_stderr_larger_than_the_pipe_buffer` … (+4 more) |
| tui/src/terminal.rs | `columns_rows_read_cached_size`, `columns_rows_read_cached_size`, `start_rejects_non_tty_and_double_start`, `refresh_size_picks_up_pty_dimensions`, `reader_loop_full_cycle_on_pty`, `reader_loop_gates_bursts_on_a_pty` … (+4 more) |
| channels/tests/channel_bin.rs | `unwritable_home_warns_and_exits_ok`, `no_channels_enabled_sigint_exits_cleanly`, `enabled_channel_with_dead_agent_sigint_exits_cleanly`, `dingtalk_enabled_with_dead_agent_sigint_exits_cleanly`, `both_channels_enabled_sigint_exits_cleanly`, `disabled_channels_sigint_exits_cleanly` … (+3 more) |
| agent/src/config/providers.rs | `auth_file_is_owner_only_models_is_not`, `upsert_restores_models_when_models_write_fails`, `upsert_restores_models_when_auth_write_fails`, `delete_restores_models_when_models_write_fails`, `delete_restores_models_when_auth_write_fails`, `delete_auth_failure_does_not_rewrite_unchanged_models` … (+1 more) |
| tui/src/clipboard.rs | `native_runner_round_trips_through_a_real_program`, `native_runner_reports_a_nonzero_exit_without_stderr`, `native_runner_reports_stderr_from_a_failing_program`, `run_native_reports_a_missing_stdin_pipe`, `run_native_reports_a_broken_stdin_write` |
| agent/src/sandbox/linux/plan.rs | `dangling_secret_symlink_is_not_treated_as_absent`, `glob_snapshot_includes_lexical_symlink_and_canonical_target`, `narrow_glob_allow_shadows_the_lower_glob_mount`, `glob_match_limit_fails_closed` |
| agent/tests/cli_smoke.rs | `home_flag_runs_a_second_isolated_instance`, `agent_default_mode_binds_per_user_local_socket`, `agent_shuts_down_cleanly_on_sigint`, `agent_preserves_unreadable_legacy_orphan_root` |
| desktop/src-tauri/src/agent_providers/tests.rs | `upsert_local_rolls_back_when_the_key_write_fails`, `upsert_local_rolls_back_when_models_write_fails`, `delete_local_rolls_back_when_the_auth_write_fails`, `delete_local_rolls_back_when_models_write_fails` |
| packages/rpc/src/transport.rs | `automatic_mode_never_falls_back_to_the_shared_tcp_port`, `explicit_socket_env_wins_over_a_redirected_future_home`, `redirected_future_home_owns_its_endpoint_and_ignores_xdg`, `without_a_redirect_the_endpoint_keeps_its_documented_chain` |
| agent/src/rpc/session.rs | `execute_shell_bounds_inherited_pipe_lifetime`, `execute_shell_snapshot_can_be_cancelled_without_session_lock`, `execute_shell_timeout_kills_the_process_group` |
| agent/src/utils/mod.rs | `ensure_workspace_accessible_repairs_readonly_dir`, `repair_dir_permissions_noop_when_owner_bits_present`, `ensure_workspace_accessible_does_not_repair_without_flag` |
| tui/src/components/autocomplete.rs | `attachment_completions_via_fd_stub`, `attachment_completions_fall_back_to_find`, `attachment_completions_empty_when_no_searcher_works` |
| orchestration/loop/src/compat.rs | `a_dead_holder_lock_that_cannot_be_unlinked_names_the_removal`, `create_in_unwritable_dir_reports_contextual_error` |
| orchestration/loop/tests/console_drive_cov100.rs | `backfill_append_fails_on_read_only_goal_dir`, `supervisor_propose_append_fails_on_read_only_goal_dir` |
| tui/src/paste.rs | `the_real_spawner_captures_a_tool_that_prints_text`, `the_real_spawner_reports_a_tool_that_is_not_installed` |
| agent/src/rpc/commands/run_control_tests.rs | `shell_rpc_releases_session_lock_and_abort_stops_process` |
| agent/src/rpc/commands/session_lifecycle_tests.rs | `fork_and_clone_report_save_errors` |
| agent/src/sandbox/linux/glob_scan.rs | `matching_symlink_protects_target_and_broken_link_fails_closed` |
| agent/src/sandbox/paths.rs | `canonicalize_lenient_follows_symlinked_dir_for_new_files` |
| agent/src/sandbox/windows/capability.rs | `save_atomic_cleans_up_temporary_on_write_failure` |
| agent/src/session/manager.rs | `load_rejects_session_with_only_blank_lines` |
| desktop/src-tauri/src/agent_supervisor.rs | `cleanup_windows_sandbox_permissions_is_a_noop_off_windows` |
| desktop/src-tauri/src/auth_store.rs | `write_sets_owner_only_permissions` |
| desktop/src-tauri/src/remote_host/files.rs | `staging_dir_is_owner_only` |
| desktop/src-tauri/src/remote_host/session_files.rs | `symlinks_cannot_escape_the_root` |
| desktop/src-tauri/src/store/cleanup.rs | `orphan_subdirs_sweep_non_utf8_names_as_orphans` |
| desktop/src-tauri/src/store/util.rs | `count_workspace_files_walks_dirs_skipping_symlinks_and_cycles` |
| desktop/src-tauri/src/store/workspaces.rs | `create_workspace_resolves_an_aliased_path_to_one_workspace` |
| desktop/src-tauri/src/terminal/cwd.rs | `an_unreadable_directory_is_not_accessible` |
| desktop/src-tauri/tests/headless_cli.rs | `terminal_signals_cancel_startup_and_leave_the_external_endpoint_alone` |
| orchestration/loop/src/canary/mod.rs | `check_root_writable_reports_failure_on_readonly_root` |
| orchestration/loop/src/console.rs | `followthrough_refresh_read_only_ledger_hits_error_edge` |
| orchestration/loop/src/work_items/task_lease.rs | `dead_holder_todos_detects_stranded_open_leases` |
| orchestration/loop/tests/agent_client_executor.rs | `execute_turn_validator_pass_and_fail` |
| orchestration/loop/tests/agent_run_drive.rs | `run_validator_pass_completes_todo` |
| orchestration/loop/tests/console_drive_cov100c.rs | `notify_dead_holders_returns_when_connect_fails` |
| orchestration/loop/tests/console_final_console2.rs | `scheduler_tick_heartbeat_append_fails_on_read_only_ledger` |
| orchestration/loop/tests/console_subprocess_drive.rs | `worker_bridge_read_only_ledger_hits_decision_record_error` |
| orchestration/loop/tests/console_sweep.rs | `notify_dead_holders_pushes_to_registered_supervisor` |
| orchestration/loop/tests/reliability_contract.rs | `detached_watchdog_reports_death_of_the_only_worker` |
| orchestration/loop/tests/write_scope_dependencies.rs | `nonexistent_files_below_symlinked_parents_have_the_same_scope` |
| tui/src/app.rs | `normalize_path_resolves_dotdot_and_clamps_at_root` |
| tui/src/crash.rs | `raw_write_stderr_writes_bytes` |
| tui/src/external_editor.rs | `create_draft_file_reports_unwritable_directory` |
| tui/src/index.rs | `run_interactive_fails_fast_without_tty` |

Gated non-test helpers (not tests themselves, but unavailable on this host,
so any Windows path that would have used them cannot be exercising them):

* agent/src/sandbox/mod.rs: `unix_shell`, `resolve_unix_shell`, `unix_shell_display_name`, `probe_legacy_bash`, `legacy_bash_from_exit_code`, `legacy_bash_probe`
* tui/src/index.rs: `run_interactive_terminal_init_failure`, `with_null_stdin`, `install`, `drop`, `interactive_loop_runs_and_ctrl_c_quits`, `new`
* packages/rpc/src/transport.rs: `local_socket_path`, `local_socket_path_from`, `local_endpoint_label`, `connect_local`, `drop`, `drop`
* cli/src/commands/init.rs: `create_symlink`, `realpath_non_enoent_error_propagates`, `installs_builtins_and_creates_idempotent_macos_links`, `installs_builtins_and_creates_links_on_linux`, `links_future_when_sibling_agent_is_missing`, `init_command_with_defaults_errors_on_test_binary`
* cli/src/browser/safari/safari_manager.rs: `drop`, `drop`, `drop`, `drop`, `set_driver_override`, `start_launch_spawn_failure`
* agent/src/config/providers.rs: `skip_if_root`, `make_readonly`, `upsert_models_write_fails_body`, `upsert_auth_write_fails_body`, `delete_models_write_fails_body`, `delete_auth_write_fails_body`
* cli/src/commands/browser_tools.rs: `drop`, `drop`, `start_launch_becomes_reachable_reports_started`, `start_launch_never_reachable_reports_starting`, `start_default_profile_dir_uses_scanned_port`
* cli/src/commands/doctor.rs: `doctor_detects_agent_binary_not_running_as_issue`, `doctor_captures_partial_version_from_hanging_binary`, `doctor_component_without_version_output_falls_back`, `doctor_sessions_dir_unreadable`, `doctor_config_files_unreadable`
* desktop/src-tauri/src/terminal/pty.rs: `pid_running`, `signal_pid`, `signal_group`, `session_members`, `session_members_via_ps`
* agent/src/tools/mod.rs: `kill_process_group`, `scoped_workspace_rejects_symlink_escape`, `run_shell_abort_kills_grandchildren`, `shell_escalated_prequest_approval_paths`
* tui/src/crash.rs: `raw_write_stderr`, `raw_write_fd`, `start`, `drop`
* tui/src/terminal.rs: `install`, `drop`, `install`, `drop`
* agent/src/sandbox/windows/capability.rs: `skip_if_root`, `make_readonly`, `save_atomic_cleanup_body`
* orchestration/loop/src/validator.rs: `large_stderr_is_drained_and_retention_is_bounded`, `cancelling_validation_reaps_the_owned_process`, `hung_child_and_inherited_pipe_are_bounded`
* tui/src/components/autocomplete.rs: `with_stubbed_path`, `argv_echoing_exe`, `attachment_context`
* channels/src/providers/slack_tests.rs: `socket_mode_fails_when_the_ack_cannot_be_written`, `socket_mode_fails_when_the_pong_cannot_be_written`
* channels/tests/channel_bin.rs: `sigint_shutdown_case`, `sigint_case_with_home`
* cli/src/utils/files.rs: `which_finds_the_host_shell`, `which_empty_output_is_none`
* desktop/src-tauri/src/config_io.rs: `create_owner_only`, `set_owner_only`
* desktop/src-tauri/src/terminal/shell.rs: `account_login_shell`, `known_fallbacks`

**Consequence for the claim.** A green local measurement says the *measured*
code is exercised on Windows; it does not say these gated bodies are correct.
The Linux CI job is where they first execute, so a failure there is a real
failure, not noise. Recorded so the final report cannot imply more than it tested.
