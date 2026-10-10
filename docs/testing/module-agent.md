# future-agent — coverage, waivers and test-quality record

Goal `cov-100-multidim` · module **future-agent** (`agent/`) · branch `test/cov100`
(worktree `.worktrees/cov100`) · host **Windows x86_64**, rustc pinned by
`rust-toolchain.toml` · measured 2026-09-26 · session
`20260926-012916-281f90c369f74d8c808f5445498b51c8`.

Evidence literals for this handoff: **lines-100-or-waived** (not yet satisfied — see
§6), **dimensions** (§4), **weak-tests-fixed** (§5).

## 1. Measurement (exact commands)

`cargo llvm-cov` 0.9.1 has **no `--target-dir` flag**; the plan's command form is
rejected (`error: invalid option '--target-dir'`). The equivalent private target dir
is `CARGO_TARGET_DIR`:

```powershell
$env:CARGO_TARGET_DIR = "target/cov-agent"
Get-ChildItem target/cov-agent -Recurse -Filter *.profraw | Remove-Item -Force   # see §7.1
cargo llvm-cov -p future-agent --no-report
cargo llvm-cov report --json --output-path coverage/agent-report.json
python .future/cov100/verify.py crate future-agent 99.99 coverage/agent-report.json
```

Report: `coverage/agent-report.json` (private; never `coverage/llvm-cov-full.json`).

## 2. Result — before → after (measured, not estimated)

| run | covered / total lines | line % | uncovered lines | files with gaps |
|---|---|---|---|---|
| baseline (this worktree, start of segment) | 56067 / 59047 | 94.9500 % | 3253 | 84 |
| after | **57455 / 59782** | **96.1075 %** | **2782** | 88 |

Delta: **+1388 covered lines, +1.16 pp**. The denominator grew by 735 lines because
un-gating in-file `#[cfg(test)]` test modules (§3) added *their* lines to the
measurement (they execute, so they land in the numerator as well).

Baseline cross-check: the supervisor's whole-workspace report
(`coverage/llvm-cov-full.json`, 2026-09-26 00:51) has the same shape for this crate
(agent ≈ 94.97 %), so the before/after pair is comparable.

## 3. What changed (files + the tests that now run)

### 3.1 `agent/src/sandbox/linux/plan.rs` — test module un-gated (+14 tests on Windows)

`#[cfg(all(test, unix))] mod tests` → `#[cfg(test)] mod tests`, and the test-only
`expand_glob` helper likewise. The plan is a pure path computation; only two cases
depend on POSIX path spelling and stay `#[cfg(unix)]` (see below). Newly executing on
Windows:

`oversized_literal_mount_plan_fails_before_helper_execution`,
`roots_reopen_and_hard_deny_are_compiled`,
`missing_and_existing_secret_masks_are_exclusive`,
`missing_guard_under_read_only_home_is_omitted_not_mounted`,
`missing_guard_under_read_only_mask_is_omitted`,
`missing_guard_under_writable_reopen_is_omitted_for_post_detection`,
`separate_read_and_write_rules_use_only_the_opaque_mask`,
`path_inspection_errors_are_not_missing_paths`,
`read_allow_reopens_read_only_and_write_only_reopen_fails_closed`,
`malformed_layer_matcher_and_relative_path_fail_closed`,
`glob_matcher_supports_recursive_and_segment_wildcards`,
`compiled_scanner_matches_existing_linux_glob_semantics`,
`reaching_glob_depth_limit_fails_closed`,
`same_layer_first_match_wins_for_overlapping_write_rules`.

Still `#[cfg(unix)]`, with the reason recorded inline: `narrow_glob_allow_shadows_the_lower_glob_mount`
and `glob_match_limit_fails_closed` — the scanner matches a pattern's **literal text**
against walked path strings, and those fixtures spell the last segment with a POSIX
`/` (`work/*.pem`). A Windows walk spells the same path with `\`, so the two tests
cannot observe the path form the Linux helper observes. `glob_match_limit_fails_closed`
also gained a diagnostic (`expected match-limit failure, got {outcome:?}`) instead of a
bare `matches!` — no assertion was weakened.

### 3.2 `agent/src/sandbox/linux/request.rs` — test module un-gated (+7 tests)

`#[cfg(all(test, unix))]` → `#[cfg(test)]`; fixtures now use
`test_support::host_absolute_path` instead of hard-coded POSIX paths, because the code
under test validates **host** paths (`Path::is_absolute`). Assertions are unchanged:
`omitted_missing_paths_round_trip_and_are_validated`,
`round_trip_is_versioned_and_structured`,
`rejects_unknown_version_unsafe_paths_and_nul_argv`,
`inner_requires_unique_non_stdio_fds_and_identity`,
`legacy_missing_mounts_are_rejected_in_both_phases`,
`report_fd_is_outer_only_and_cannot_overlap_mount_fds`,
`request_size_mount_count_and_unknown_fields_are_bounded`.

### 3.3 `agent/src/sandbox/linux/runner.rs` — test module un-gated (+5 tests)

Same treatment. `large_request_is_carried_out_of_band_instead_of_argv` needed a real
platform fix: a 90 KiB command is legal argv on unix, but Windows carries the command
as a base64 UTF-16 PowerShell script (~2.7 argv bytes per command byte), so the same
command exceeds the 96 KiB argv budget. The test now derives the command size from the
host's own `shell_argv` cost and asserts `request.argv == shell_argv(&command)` (a
stronger, spelling-independent claim) instead of `argv.last().len() == command.len()`.

### 3.4 `agent/src/lib.rs` — new shared test helper

`test_support::host_absolute_path(&str) -> PathBuf`: the host-spelling absolute path for
a POSIX-spelled argument (`C:\...` on Windows, unchanged on unix). Used by 3.2 and 3.3.

### 3.5 Weak tests rewritten (see §5)

`agent/src/rpc/session.rs` (10 rewritten, 1 duplicate deleted) and
`agent/src/sandbox/mod.rs` (2 rewritten).

## 4. Dimensions (matrix rows for future-agent)

| dimension | evidence (tests that make the claim real) |
|---|---|
| boundary | `sandbox::linux::request::tests::request_size_mount_count_and_unknown_fields_are_bounded` (MAX_MOUNTS+1 mounts, over-long base64 payload, unknown JSON field); `sandbox::linux::plan::…::oversized_literal_mount_plan_fails_before_helper_execution`; `runner::…::large_request_is_carried_out_of_band_instead_of_argv`; `glob_scan` budget tests (POSIX only); `tools::tests` shell-output tail at the 2000-byte mark; `glob_matcher_supports_recursive_and_segment_wildcards` includes `密.pem` (Unicode/CJK) |
| error-path | `request::…::rejects_unknown_version_unsafe_paths_and_nul_argv` (version/safety/argv); `inner_requires_unique_non_stdio_fds_and_identity` (duplicate fd); `legacy_missing_mounts_are_rejected_in_both_phases`; `report_fd_is_outer_only_and_cannot_overlap_mount_fds`; `runner::prepare` probe-receipt validation (now executed: `InvalidProbeReceipt` arms); `plan::…::path_inspection_errors_are_not_missing_paths`, `malformed_layer_matcher_and_relative_path_fail_closed` |
| concurrency | `agent/tests/sqlite_storage.rs::concurrent_clones_share_one_ordered_writer`; `rpc::session::tests::scheduler_worker_starts_queued_run_after_completion` + `repeated_manual_compaction_after_restart_uses_persisted_checkpoint`; `sandbox::windows::runner::tests::active_capabilities_block_only_their_own_state_file`; `sandbox::windows::integration_tests::stale_capability_gc_waits_for_active_process_tree` |
| property | `plan::…::glob_matcher_supports_recursive_and_segment_wildcards` + `compiled_scanner_matches_existing_linux_glob_semantics` (table-driven over the pattern grammar); `request::…::round_trip_is_versioned_and_structured` and `omitted_missing_paths_round_trip_and_are_validated` (encode → decode invariant); `models::future` / `session::projection` round-trips (pre-existing) |
| platform-cfg | this segment's main contribution: 26 previously unix-only tests now compile and run on Windows and unix via `test_support::host_absolute_path`; `runner::…::large_request_is_carried_out_of_band_instead_of_argv` now models the Windows `-EncodedCommand` cost; `sandbox::windows::*` integration tests (66 tests, run unelevated on this host); the two remaining POSIX-only `plan.rs` tests are `cfg(unix)` with an inline reason |
| serialization | `request::…::round_trip_is_versioned_and_structured` (versioned wire struct); `request_size_mount_count_and_unknown_fields_are_bounded` (unknown field ⇒ `InvalidJson`); legacy payloads: `legacy_missing_mounts_are_rejected_in_both_phases`, and the pre-existing `agent/tests/sqlite_storage.rs::a_retired_schema_checkpoint_imports_as_an_inert_entry` / `migration_preserves_sources_payloads_and_deletion_tombstones` |

## 5. Weak-test audit — weak-tests-fixed

Method: the same detector `verify.py weak` uses (`#[test]` / `#[tokio::test]` bodies with
no assertion-ish token), scoped to `agent/`, reproduced read-only in
`coverage/weak_scan.py` because `docs/testing/weak-test-audit.md` (a separate
deliverable, not owned by this task) does not exist yet — `verify.py weak` currently
fails with `docs/testing/weak-test-audit.md missing`.

Census: **1673** test fns, **46** assertion-free at the start of the segment → **33** at
the end. **13 fixed**, no assertion weakened and no coverage deleted:

| before | after | what now fails if the code is wrong |
|---|---|---|
| `rpc/session.rs::add_session_rule_does_not_panic` | `add_session_rule_records_a_read_only_allow_and_reaches_the_live_rule_set` | rule count/decision/access, the rule reaching the *live* `RuleSet` session layer (index 2 of `profile_layers()`), and a second injection appending rather than replacing |
| `set_permission_level_invalid` | `set_permission_level_invalid_value_is_stored_verbatim` | documents the real contract: no validation here, the value read back is the value set |
| `set_system_prompt_updates` | same name | `loop.system_prompt` **and** `loop.config.system_prompt` (the copy sent to the provider) |
| `append_system_prompt_appends` | same name | `base\nappended`, plus the empty-prompt arm having no leading newline |
| `set_tools_filters` | `set_tools_keeps_only_the_requested_catalogue_entries` | catalogue intersection, and an unknown name cannot smuggle a tool in |
| `disable_tools_clears` | same name | populate via `set_tools` → assert non-empty → `disable_tools()` → assert empty |
| `disable_builtin_tools` | same name | same round-trip |
| `set_ephemeral_toggles` | **deleted** — exact duplicate of the stronger pre-existing `set_ephemeral` (which already asserts both directions) | — |
| `fork_does_not_panic` + `delete_session_does_not_panic` | `fork_and_delete_leave_the_live_session_untouched` | both Tier-1 helpers return `Ok` **and** leave the live message list unchanged |
| `set_sandbox_policy_updates` | same name | default is `None`; setting stores the tier; a later policy replaces the earlier one |
| `sandbox/mod.rs::rule_set_returns_reference` | `rule_set_returns_the_live_rule_set_by_reference` | pointer identity across calls (a per-call re-resolve would drop session injections) + `evaluate(workspace file, Read) == Allow` |
| `shell_supports_chain_operators_returns_bool` | `shell_supports_chain_operators_is_true_for_posix_shells` | asserts the POSIX arm; the Windows arm is derived from the detected shell and is honestly **not** a claim here (see the open item below) |

Remaining 33 assertion-free tests (heuristic), not fixed in this segment:

| file(s) | n | assessment / next step |
|---|---|---|
| `config/providers.rs` | 8 | rollback/restore arms driven by a deliberately failing writer; the tests do call the code and would panic on a wrong error, but they assert nothing — strengthen by asserting the restored file bytes and the returned `Err` variant |
| `sandbox/mod.rs` | 4 | `hydrate_*` arms (`ignores_dump_without_marker`, `skips_when_shell_spawn_fails`, `times_out_and_kills_hung_shell`) — strengthen by asserting `$SHELL`/env is unchanged and the typed timeout outcome |
| `models/mod.rs` | 3 | `get_default_model_returns_something`, `model_accepts_images_returns_bool`, `registry_resolve_scope_with_star` are explicit "may be None/empty" smoke tests; replace with assertions against a constructed `Registry` |
| `rpc/prompt_helpers.rs` | 3 | `approve_tool_path_*` no-op arms — assert the mutated gate/`approved` list |
| `session/persistence.rs` | 3 | assert the persisted bytes / returned error after the injected failure |
| `llm/schema.rs` 2, `llm/adapters/anthropic.rs` 1 | 3 | `expect_*_rejects_other_protocols` — assert the returned `Err` |
| `cli/shutdown.rs` 1, `models/future.rs` 1, `rpc/approval.rs` 1, `sandbox/windows/capability.rs` 1, `sandbox/windows/process.rs` 1, `skills/manager.rs` 1, `tools/mod.rs` 1, `utils/mod.rs` 1 (`is_tty_returns_bool`), `agent/tests/session_load_test.rs` 1 | 9 | each pinned to a single smoke assertion; `is_tty_returns_bool` in particular is assertion-free and needs a piped-stdout child process to be made real |

## 6. Waiver ledger and remaining-gap census

Total uncovered (segment report): **2782 lines in 88 measured files**. Gate status:
**lines-100-or-waived is not satisfied** — 3 files carry a real waiver category, 85 are
recorded as `open-gap` (testable, no waiver claimed), because mislabelling testable code
as unreachable would be worse than an honest gap.

### 6.1 Waived (every line named)

| file | uncovered | category | lines | reason |
|---|---|---|---|---|
| `agent/src/sandbox/linux/glob_scan.rs` | 63 | `unreachable-in-this-environment` | 67, 75, 77-79, 106-108, 112, 120-121, 129-131, 192, 194, 197, 199, 204-206, 219, 223-225, 234, 236, 238-240, 242, 245, 253, 256-257, 261-265, 267-269, 271-275, 278-283, 285, 287-289, 306, 309, 311, 317, 323 | The scanner's grammar and its fixtures are POSIX path-spelled: `Pattern::compile` splits on `/` and `compile_matcher` emits `[^/]*`, and `scan_root` derives the walk root from the pattern text. A Windows walk returns `\`-spelled paths, so these arms (match/descend/symlink/budget decisions) cannot be driven from a Windows-spelled fixture — proved by the two `cfg(unix)` tests kept in `plan.rs` (§3.1). A Linux-host measurement covers them; on this host they are unreachable in-environment. |
| `agent/src/sandbox/linux/violation.rs` | 3 | `unreachable-by-construction` | 285-287 | `impl std::io::Write for Closed` in the test module: `write` always fails with `BrokenPipe`, so `flush` is never reached by the code under test. It cannot be deleted — `io::Write` requires a `flush` implementation. Proved by the surrounding test, which asserts the emit failure path. |
| `agent/src/sandbox/linux/post_scan.rs` | 1 | `unreachable-in-this-environment` | 72 | Body line of a `#[cfg(unix)]`-gated test fixture inside `#[cfg(test)] mod` that *is* compiled here; the test itself is excluded from the build on Windows, so its first statement has no execution count. |
| `agent/build.rs`, `agent/src/agent/events.rs`, `agent/src/llm/stream_wait_tests.rs`, `agent/src/models/reasoning_tests.rs`, `agent/src/runtime/mod.rs`, `agent/src/sandbox/seatbelt.rs`, `agent/src/session/compaction_ops_tests.rs`, `agent/src/session/mod.rs`, `agent/src/compaction/durable/tests.rs`, `agent/src/compaction/semantic/evidence/tests.rs`, `agent/src/rpc/commands/dispatcher_tests.rs`, `agent/src/rpc/commands/observability_tests.rs`, `agent/src/rpc/commands/providers_tests.rs`, `agent/src/rpc/commands/run_control_tests.rs`, `agent/src/rpc/commands/session_lifecycle_tests.rs`, `agent/src/rpc/commands/settings_tests.rs`, `agent/src/rpc/session_prompt/build_user_message_tests.rs`, `agent/src/rpc/session_prompt/tests.rs`, `agent/src/sandbox/linux/helper.rs`, `agent/src/sandbox/linux/mod.rs`, `agent/src/sandbox/windows/integration_tests.rs` | absent from the report | `platform-unmeasured` | — | 21 source files are absent from `coverage/agent-report.json` (verified with `verify.py blindspot future-agent docs/testing/module-agent.md`). Two reasons: (a) platform-gated code never compiled here — `seatbelt.rs` is macOS-only, `helper.rs` is `#![cfg(target_os = "linux")]`; (b) translation units that llvm-cov does not report at all: the 12 standalone `*_tests.rs` files plus `mod`-only/`build.rs` files (0 `*_tests.rs` entries among the report's 107 files). Neither is numerator nor denominator, so a Windows "100 %" would never prove these compile or run. |

### 6.2 Open gaps (no waiver claimed) — full census

Bucket legend for the `next step` column: **B** = a whole function/feature has no test
yet (needs a new behaviour test); **A** = error/edge arms (`?`, `map_err`, non-happy
branch) of functions whose main body is already covered (needs targeted error-path
tests); **C** = error/edge arms in the I/O, store or persistence layer (needs an
injected I/O failure); **D** = Windows-sandbox-specific path — testable on this host,
the restricted-token machinery runs unelevated here (§7.4).

| file | uncovered | next step |
|---|---|---|
| `agent/src/skills/manager.rs` | 179 | B — install/sync/reconcile failure arms; the existing tests already build a stub HTTP catalogue server, so extend those fixtures |
| `agent/src/tools/mod.rs` | 133 | D — ~95 of these are `spawn_windows_restricted_shell`, the Sandbox-tier shell path |
| `agent/src/agent/run_loop.rs` | 116 | B — retry/truncation/cancel error arms |
| `agent/src/rpc/session.rs` | 110 | A — error/edge arms remaining after this segment's rewrite |
| `agent/src/session/sqlite_store.rs` | 98 | C |
| `agent/src/session/legacy_import.rs` | 94 | B — legacy JSONL import arms |
| `agent/src/session/manager.rs` | 85 | A |
| `agent/src/llm/adapters/openai_responses.rs` | 79 | A — adapter event/error arms |
| `agent/src/session/database.rs` | 76 | C |
| `agent/src/sandbox/windows/runner.rs` | 74 | D — capability lease/revoke/GC arms |
| `agent/src/cli.rs` | 65 | A |
| `agent/src/llm/mod.rs` | 63 | A |
| `agent/src/rpc/protocol.rs` | 60 | A |
| `agent/src/sandbox/mod.rs` | 55 | A |
| `agent/src/skill_reco/mod.rs` | 52 | A |
| `agent/src/rpc/commands/settings.rs` | 50 | A |
| `agent/src/compaction/semantic/evidence.rs` | 49 | A |
| `agent/src/types/mod.rs` | 49 | A |
| `agent/src/sandbox/windows/process.rs` | 47 | D |
| `agent/src/compaction/semantic.rs` | 45 | A |
| `agent/src/rpc/approval.rs` | 45 | A |
| `agent/src/rpc/commands/session_lifecycle.rs` | 44 | A |
| `agent/src/rpc/session_prompt.rs` | 43 | A |
| `agent/src/cli/shutdown.rs` | 42 | A |
| `agent/src/config/providers.rs` | 42 | C |
| `agent/src/rpc/commands/mod.rs` | 41 | A |
| `agent/src/session/history_query.rs` | 37 | A |
| `agent/src/session/compaction_ops.rs` | 36 | A |
| `agent/src/session/fork.rs` | 36 | A |
| `agent/src/rpc/commands/session_title.rs` | 35 | A |
| `agent/src/session/history_index.rs` | 35 | A |
| `agent/src/rpc/mod.rs` | 33 | A |
| `agent/src/sandbox/linux/plan.rs` | 33 | A — note: lines 341-342, 736, 738, 746 are fixture/helper lines of the two POSIX-only tests above (same bucket as the glob scanner), but the other ~27 are testable error arms, so this file is deliberately **not** waived as a whole |
| `agent/src/sandbox/windows/token.rs` | 31 | D |
| `agent/src/sandbox/linux/probe.rs` | 30 | A — probe parse/version/error arms |
| `agent/src/skills/registry.rs` | 30 | A |
| `agent/src/session/persistence.rs` | 28 | C |
| `agent/src/sandbox/linux/request.rs` | 26 | A |
| `agent/src/llm/adapters/anthropic.rs` | 24 | A |
| `agent/src/llm/adapters/openai_chat.rs` | 24 | A |
| `agent/src/models/mod.rs` | 24 | A |
| `agent/src/rpc/commands/observability.rs` | 24 | A |
| `agent/src/rpc/commands/skills.rs` | 23 | A |
| `agent/src/session/records.rs` | 23 | A |
| `agent/src/sandbox/windows/audit.rs` | 22 | D |
| `agent/src/session/tools.rs` | 21 | A |
| `agent/src/grpc/mod.rs` | 20 | A |
| `agent/src/utils/mod.rs` | 20 | A |
| `agent/src/runtime/scheduler_queue.rs` | 19 | A |
| `agent/src/compaction/durable.rs` | 17 | C |
| `agent/src/llm/schema.rs` | 16 | A |
| `agent/src/rpc/commands/providers.rs` | 14 | A |
| `agent/src/sandbox/windows/acl.rs` | 14 | D |
| `agent/src/logfile.rs` | 13 | C |
| `agent/src/skills/mod.rs` | 13 | A |
| `agent/src/rpc/run_snapshot.rs` | 12 | A |
| `agent/src/sandbox/windows.rs` | 11 | D |
| `agent/src/models/future.rs` | 9 | C |
| `agent/src/rpc/prompt_helpers.rs` | 9 | A |
| `agent/src/session/summary.rs` | 9 | A |
| `agent/src/sandbox/windows/capability.rs` | 8 | D |
| `agent/src/llm/adapters/mod.rs` | 7 | A |
| `agent/src/prompt/mod.rs` | 7 | A |
| `agent/src/runtime/run_state.rs` | 7 | A |
| `agent/src/session/checkpoint.rs` | 7 | A |
| `agent/src/agent/mod.rs` | 6 | A |
| `agent/src/compaction/mod.rs` | 6 | A |
| `agent/src/runtime/session_runtime.rs` | 6 | A |
| `agent/src/sandbox/windows_request.rs` | 6 | D |
| `agent/src/tools/cmd_exe_rewrite.rs` | 6 | A |
| `agent/src/sandbox/paths.rs` | 5 | A |
| `agent/src/session/projection.rs` | 5 | A |
| `agent/src/sandbox/linux/report.rs` | 4 | C — helper report write/read arms (oversize reject, seek/take) |
| `agent/src/sandbox/linux/runner.rs` | 4 | A |
| `agent/src/session/display.rs` | 4 | A |
| `agent/src/auth/mod.rs` | 3 | A |
| `agent/src/config/mod.rs` | 3 | A |
| `agent/src/sandbox/rules.rs` | 3 | A |
| `agent/src/session/repair.rs` | 3 | C |
| `agent/src/engine/mod.rs` | 2 | A |
| `agent/src/lib.rs` | 2 | A — the `test_support` helper arms added this segment |
| `agent/src/llm/sse.rs` | 1 | A |
| `agent/src/rpc/commands/run_control.rs` | 1 | A |
| `agent/src/sandbox/windows_plan.rs` | 1 | A |
| `agent/src/session/run_journal.rs` | 1 | C |
| **total** | **2782** | 3 files waived (67 lines) + 85 files open (2715 lines); per-line ranges are reproducible with `python .future/cov100/verify.py uncovered future-agent coverage/agent-report.json` |

## 7. Findings worth acting on

### 7.1 Stale `*.profraw` corrupts a reused private target dir (measurement bug, not a product bug)

`cargo llvm-cov --no-report` leaves the previous build's `*.profraw` files in the target
dir. After a source change the same test binary keeps its **file name** but gets a new
counter layout, so `llvm-cov` merges profiles from two different builds. Observed
effect: `sandbox/linux/plan.rs` reported **5 / 754** covered lines immediately after all
14 of its tests passed, while the identical run with `*.profraw` deleted reported
**733 / 754**. Every number in this document was taken after deleting `*.profraw`.
Recommendation: add the delete to the supervisor's measurement script. The first
"before" measurement and the supervisor's baseline both predate the discovery but agree
with each other (84 files, ≈3250 uncovered lines), so this segment's before/after pair
is unaffected.

### 7.2 Latent portability hazard: Linux glob rules match nothing on Windows

`glob_scan::compile_matcher` + `scan_root` treat the pattern as POSIX-spelled and compare
it literally against walked path strings; a Windows-spelled walk path can never match a
`/`-separated pattern segment (`[^/]*` plus a required literal `/`). Reachability:
`sandbox::linux` compiles everywhere but is only *entered* when the Linux probe reports
available, so this is latent on Windows, not a live bug. Reported instead of fixed —
normalising separators here would change security-sensitive matching semantics, which
is outside a coverage task.

### 7.3 llvm-cov omits test-only translation units

0 of the report's 107 files are `*_tests.rs`; all 12 standalone test files (and
`mod`-only files) are absent. The crate percentage therefore covers *production code +
in-file test modules* only. Comparable before/after, but not comparable with a crate
whose tests live in `tests/` only.

### 7.4 Windows sandbox gaps are test gaps, not environment limits

66 `sandbox::windows::*` tests (restricted token, ACL plan, capability lease, GC) pass
unelevated on this host, so the 214 uncovered lines under `sandbox/windows/` are missing
tests rather than an environmental limit. Do not treat them as waivable without
evidence that a specific arm needs a privilege this host lacks.

## 8. Handoff state

- Branch `test/cov100`, nothing committed by this task (supervisor commits at the
  checkpoint). Edited by this segment: `agent/src/lib.rs`, `agent/src/rpc/session.rs`,
  `agent/src/sandbox/mod.rs`, `agent/src/sandbox/linux/plan.rs`,
  `agent/src/sandbox/linux/request.rs`, `agent/src/sandbox/linux/runner.rs`,
  `docs/testing/module-agent.md`.
- All 1847 lib tests + 37 integration/smoke tests pass (`coverage/llvmcov-final.log`);
  no test was skipped, ignored or weakened, and no production branch was made
  unreachable.
- Next useful checks, in order: (1) `tools/mod.rs::spawn_windows_restricted_shell`
  (95 lines, one test); (2) `skills/manager.rs` (179 lines, existing stub-server
  fixtures); (3) the 33 remaining assertion-free tests in §5; (4) re-measure with
  `*.profraw` cleared, then let the gate report which of the 85 open files still need
  tests. `verify.py crate future-agent 99.99` cannot pass until every gap file is either
  covered or carries a real category.
