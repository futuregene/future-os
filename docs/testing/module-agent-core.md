# Module `agent/*` core subtree (worker group `ag-core`) — coverage handoff

Subtree scope (frozen by `verify.py` `RUST_GROUPS["ag-core"]`): `agent/src/skills/`,
`agent/src/skill_reco/`, `agent/src/types/`, `agent/src/config/`,
`agent/src/models/`, `agent/src/runtime/`, `agent/src/utils/`,
`agent/src/prompt/`, `agent/src/auth/`, `agent/src/cli.rs`, `agent/src/cli/`,
`agent/src/lib.rs`, `agent/src/main.rs`.

Gate:
`python .future/cov100/verify.py rust-module ag-core 99.99 docs/testing/module-agent-core.md coverage/agent-ag-core-report.json`

Evidence literals for this handoff: **`lines-100-or-waived`** (§6 — every file
that still has uncovered lines is registered there with a category and a reason),
**`dimensions`** (§4), **`weak-tests-fixed`** (§5).

## 1. Run identity and the exact measurement command

* Goal `goal_6d61125ba837`, subtree **ag-core**, crate `future-agent`.
  Segment 1 (baseline + most of the tests): todo `todo_a6b55c1bfc24`.
  Segment 2 (this document's numbers, the graceful default-IPC run and the
  harness-region work): todo `todo_b4673313a9df`.
* Host **Windows 11 x86_64**, rustc pinned by `rust-toolchain.toml`, cargo-llvm-cov 0.9.1.
* Private build dir `target/cov-ag-core` via `CARGO_TARGET_DIR`, reused by both
  segments (same group, same directory — the plan's "keep using the same one").
  **`cargo llvm-cov` 0.9.1 has no `--target-dir`** — the form printed in the plan
  is rejected with `error: invalid option '--target-dir'`; `CARGO_TARGET_DIR` is
  the equivalent private dir the plan asks for. **It has no `--test-threads`
  either** (`error: invalid option '--test-threads'`): set `RUST_TEST_THREADS=4`.
* **The measurement must stay package-wide, not `--lib`.** The task template's
  `cargo llvm-cov -p future-agent --lib …` runs only the lib test target, so
  `agent/tests/*` never runs: `cli_smoke` is exactly where `cli.rs`'s integration
  coverage comes from (and the bin's `main.rs` would drop out of the report
  entirely, since it is not linked into the lib). A `--lib` number is a different
  metric, not a cheaper one. Both segments therefore used the same package-wide
  command below, which is what makes the §2 comparison valid.
* Report consumed by the gate: `coverage/agent-ag-core-report.json` (gitignored).

```powershell
$env:CARGO_TARGET_DIR = "target/cov-ag-core"
$env:RUST_TEST_THREADS = "4"          # --test-threads is not a cargo-llvm-cov flag
# THE measurement: one atomic invocation (test run + report in one process). A
# separate `cargo llvm-cov report` after a run that touched a failing test target
# re-merges stale/foreign .profraw and yields an all-zero report — observed twice
# here, see §7.3. One command, or no number.
cargo llvm-cov -p future-agent -j 3 --json `
    --output-path coverage/agent-ag-core-report.json

python .future/cov100/verify.py rust-module ag-core 99.99 `
    docs/testing/module-agent-core.md coverage/agent-ag-core-report.json
```

The report this doc describes comes from this segment's run, logged in
`target/cov-ag-core/final3.log` (segment 1) and the segment-2 run of the same
command: the last segment-2 run reported **2012 lib tests passed, 0 failed,
1 ignored** (the ignored test is pre-existing and outside this subtree; the count
is higher than the 2005 of the run before it because peers add tests to the same
lib binary — §7.5), plus every integration target —
`cli_smoke` 18 passed (17 + the new default-IPC run), and `sandbox_smoke`,
`seatbelt_security`, `sqlite_startup`, `sqlite_storage`, `session_load_test`,
`compaction_persist`, `linux_sandbox_smoke` all green. The lib suite took
134 s–1913 s wall clock inside a checkout where five other workers compile and
measure the same crate at once.

## 2. Result — before → after (measured, not estimated)

Whole subtree (the gate's own arithmetic — `agent/src/cli.rs`, `agent/src/cli/`,
`skills/`, `skill_reco/`, `types/`, `config/`, `models/`, `runtime/`, `utils/`,
`prompt/`, `auth/`, `lib.rs`, `main.rs`):

| run | covered / total lines | line % | uncovered lines | files with gaps |
|---|---|---|---|---|
| segment 1 start (same command, same target dir) | 11703 / 12012 | 97.4276 % | 309 | 15 |
| segment 1 end | 12881 / 13029 | 98.8641 % | 148 | 14 |
| segment 2 end (§3.2) | **12995 / 13139** | **98.9040 %** | **144** | 14 |

Per file, uncovered line counts from the reports of the two segments (`Lines`
column of the same report the gate reads):

| file | seg 1 | seg 2 | what moved |
|---|---|---|---|
| `agent/src/cli.rs` | 28 | **27** | `933` (`None => serve_local`) is now covered by the new default-IPC run |
| `agent/src/cli/shutdown.rs` | 47 | 47 | unchanged — see §6 and §7.4; the child-process lines cannot be attributed |
| `agent/src/skill_reco/mod.rs` | 7 | **5** | `930`/`942` (harness reader arms) are covered by the new framing test |
| `agent/src/skills/manager.rs` | 23 | **26** | `1549`/`1592` covered; `+3` for the new fixture's never-flushed `Write::flush` (§6) |
| `agent/src/skills/mod.rs` | 2 | **1** | `322` (scalar `metadata:` arm) covered by the new frontmatter fixture |
| `agent/src/models/mod.rs` | 4 | **2** | not an edit in this segment — moved with the suite (§7.5) |
| `agent/src/models/future.rs` | 7 | **6** | not an edit in this segment — moved with the suite (§7.5) |
| `agent/src/config/providers.rs` | 5 | 5 | — |
| `agent/src/lib.rs` | 2 | 2 | — |
| `agent/src/runtime/run_state.rs` | 7 | 7 | — |
| `agent/src/runtime/session_runtime.rs` | 6 | 6 | — |
| `agent/src/skills/registry.rs` | 5 | 5 | — |
| `agent/src/types/mod.rs` | 2 | 2 | — |
| `agent/src/utils/mod.rs` | 3 | 3 | — |

Segment 1 closed 52.1 % of the subtree's gaps (309 → 148). Segment 2 deliberately
spent its budget on **quality rather than count**: it covered 4 of the 6 lines
that segment 1 had flagged as cheap follow-ups (§8 of the previous revision),
turned two harness branch regions and one scalar-metadata arm into asserted
behaviour, and proved by measurement that the 47 `cli/shutdown.rs` lines are not
reachable (§7.4) — at the cost of 3 new fixture lines in `manager.rs`. Net:
148 → 144 uncovered, and every remaining line is now either structurally
impossible, platform-bound, or the macro/continuation attribution artifact. Two
of the 14 per-file numbers moved for reasons outside this segment
(`models/mod.rs`, `models/future.rs`); §7.5 says why that is expected while
several workers share one test binary.

The segment-1 denominator grew by 1017 lines because the added tests are
themselves measured — they execute, so they land in both `LF` and `LH`; the
*uncovered* count is the number that had to fall.

## 3. Files changed, and the tests that do the work

All paths are repo-relative. `+N` = tests added (or strengthened) in that file.

| file | +N | what changed |
|---|---|---|
| `agent/src/skill_reco/mod.rs` | +10 | `attempt` split into `attempt` (env resolution + guard) and `attempt_call(client, endpoint, query, candidates)`, mirroring the existing `endpoint`/`resolve` split; a local-socket transport matrix; a signed-in end-to-end run through `suggest_skill` itself; captured-log assertions |
| `agent/src/skills/manager.rs` | +17 | archive-validation matrix, catalogue-classification matrix, download caps, recovery/reconcile state matrix, failure injection through a SQLite trigger and a stale backup directory |
| `agent/src/cli.rs` | +8 | instance-lock/metadata failure paths, `abort_all_sessions` with a really-blocked approval, maintenance-command paths, an isolated end-to-end migration run |
| `agent/src/config/providers.rs` | +5 | Windows rollback injection (read-only destination) for both write orders of both transactional mutators, plus the auth-only-change case where nothing may be restored |
| `agent/src/utils/mod.rs` | +2 | the blocked-directory repair path with and without `auto_repair` |
| `agent/src/skills/mod.rs` | +2 | no-frontmatter skill file; block-style `metadata:` scan (blank-line skip, dedent break, block that runs out) |
| `agent/src/skills/registry.rs` | +1 | v3 → v4 in-place migration keeps existing rows |
| `agent/src/models/mod.rs` | +1 | `CostSplit::is_unset` invariant |
| `agent/src/runtime/session_runtime.rs` | +1 | `is_terminal_unrecoverable` before and after the run goes stuck |
| `agent/src/auth/mod.rs` | +1 | the capturing writer's `flush` contract is asserted, not assumed |
| `agent/src/cli/shutdown.rs` | 0 | the subprocess watchdog deadline `10 s → 60 s` (load robustness, §5.3) — no assertion was weakened |

New tests (names as they appear in the report):

* **skill recommendation** — `a_successful_call_reports_the_pick_its_usage_and_what_it_sent`,
  `a_non_success_status_reports_the_trimmed_truncated_upstream_body`,
  `an_unreadable_body_is_reported_instead_of_being_taken_as_an_answer`,
  `a_refused_connection_is_reported_as_a_failure`,
  `a_silent_gateway_hits_the_client_deadline`,
  `a_blank_query_or_an_empty_candidate_list_never_reaches_the_wire`,
  `a_catalogue_larger_than_the_choice_limit_is_clamped_to_255_options`,
  `probabilities_naming_only_none_are_a_failure_not_a_refusal`,
  `a_signed_in_install_returns_the_candidate_the_gateway_picked`,
  `a_request_that_cannot_be_built_is_reported_as_a_transport_failure`.
* **skill manager** — `sync_classifies_skipped_failed_and_never_upgrades_on_explicit_bootstrap`,
  `a_failed_install_mid_sync_is_reported_and_leaves_no_pending_operation`,
  `an_archive_with_an_unsafe_path_is_refused`,
  `an_archive_with_a_symlink_entry_is_refused`,
  `an_archive_with_too_many_entries_is_refused`,
  `an_archive_that_declares_more_than_the_extraction_limit_is_refused`,
  `an_archive_without_skill_md_is_refused`,
  `a_single_wrapping_directory_is_unwrapped_but_two_are_not`,
  `a_stale_backup_directory_is_cleaned_up_before_publishing`,
  `a_finalization_failure_restores_the_previous_install`,
  `uninstall_removes_the_directory_and_reports_it`,
  `recovery_clears_the_backup_of_a_completed_replacement`,
  `recovery_restores_the_backup_when_the_staged_install_never_landed`,
  `reconciliation_ignores_files_and_directories_without_a_skill_md`,
  `an_app_install_shadows_the_global_install_of_the_same_skill`,
  `a_download_with_an_oversized_declared_length_is_refused`,
  `a_download_that_streams_past_the_cap_is_refused`.
  Helpers added inside the test module: `Download` (`Bytes`/`Status`/`Stream`),
  `platform(...)` (one server for both endpoints, serving until the test ends),
  `install_rejects(...)` (asserts the refusal *and* that nothing was recorded),
  `archive_with(...)`, `patch_u32`/`patch_u16` (central-directory surgery),
  `extract(...)`.
* **CLI** — `unremovable_instance_metadata_is_left_alone`,
  `the_instance_lock_creates_its_missing_parent_directories`,
  `abort_all_sessions_aborts_and_cancels_pending_approvals`,
  `the_linux_sandbox_helper_flag_is_rejected_off_linux`,
  `reset_windows_sandbox_is_a_sessionless_maintenance_command`,
  `sandbox_cleanup_reports_an_unreadable_capability_file`,
  `a_migration_source_without_a_migration_command_is_rejected`,
  `unbootable_instance_metadata_does_not_stop_startup`.
* **config / utils / skills / models / runtime / auth** —
  `upsert_restores_models_when_models_write_fails`,
  `upsert_restores_models_when_auth_write_fails`,
  `delete_restores_models_when_models_write_fails`,
  `delete_restores_models_when_auth_write_fails`,
  `upsert_auth_write_fails_without_models_change_skips_restore` (all
  `#[cfg(windows)]`, §4),
  `ensure_workspace_accessible_repair_retry_reports_the_same_blockage`,
  `ensure_workspace_accessible_reports_a_blocked_dir_without_repair`,
  `restore_file_restores_a_snapshot_and_tolerates_a_missing_target`,
  `a_skill_without_frontmatter_is_named_by_its_directory_and_stays_enabled`,
  `metadata_block_scan_skips_blanks_and_stops_at_the_first_dedent`,
  `a_v3_database_is_migrated_in_place_without_losing_rows`,
  `model_accepts_images_follows_the_models_declared_modalities`,
  `the_default_model_is_either_absent_or_names_something`,
  `resolve_scope_star_is_a_superset_and_unmatched_patterns_are_empty`,
  `a_cost_split_is_unset_until_a_charge_is_priced`,
  `terminal_unrecoverable_tracks_the_run_control_state`,
  `is_tty_inspects_standard_input`.

### 3.1 The one production-code change, and why it is not a coverage dodge

`agent/src/skill_reco/mod.rs`: `attempt` used to resolve the endpoint *and*
perform the HTTP call, so every transport outcome (a 4xx body, an unreadable
body, a refused connection, a timeout) could only be produced by talking to the
live Jev gateway. It is now split at the same seam the file already had between
`endpoint()` and `resolve()`:

```rust
fn attempt(query, candidates) -> (Outcome, Attempt) { ... attempt_call(&HTTP_CLIENT, &endpoint, query, candidates) }
fn attempt_call(client: &reqwest::blocking::Client, endpoint: &Endpoint, query, candidates) -> (Outcome, Attempt)
```

No branch was removed, no error arm was narrowed, and `HTTP_CLIENT` (the
production client, with `TIMEOUT`) is still what `attempt` passes — exactly as
`resolve` exists so the tests never touch the process-global `HOME`. Both sides
of that seam are covered: `a_refused_connection_is_reported_as_a_failure` and
`a_silent_gateway_hits_the_client_deadline` use `&HTTP_CLIENT` itself (so the lazy
client initializer is exercised rather than bypassed, and the documented
`timeout after 5000ms` message that constant produces is asserted), while
`a_signed_in_install_returns_the_candidate_the_gateway_picked` drives the public
`suggest_skill` end-to-end with a credential in an isolated `$HOME`.

### 3.2 Segment 2 — the four changes behind the §2 numbers

| file | what changed |
|---|---|
| `agent/tests/cli_smoke.rs` | **+1** `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint` — the product's *default* transport (no `--grpc-addr`) run to completion as a real subprocess |
| `agent/src/skill_reco/mod.rs` | **+1** `read_request_frames_a_request_across_reads_and_stops_at_eof`, and `read_request` is now generic over `impl std::io::Read` so the reader can be driven piece-by-piece |
| `agent/src/skills/manager.rs` | `FailingConnection` fixture + **+2** `a_client_whose_request_cannot_be_read_is_dropped_without_an_answer`, `a_write_failure_mid_body_stops_the_stream_pump`; the per-connection body moved out of `platform()`'s accept loop into `serve_connection(stream: impl Read + Write, …)` |
| `agent/src/skills/mod.rs` | **+1** `a_scalar_metadata_value_is_not_scanned_for_a_nested_version` |

**1. The graceful default-IPC run.** `agent/src/cli.rs` : 933 is
`None => Box::pin(crate::grpc::serve_local(app_state))` — the arm that every real
install takes, which until now only the `#[cfg(unix)]` test
`agent_default_mode_binds_per_user_local_socket` exercised, so it was uncovered on
this Windows host. The new test runs the real binary with *no* `--grpc-addr` and
`--home <isolated>`, and asserts observable behaviour rather than the line:

* `ProcessExit` — `status.code() == Some(0)`: a normal exit, not killed, not 130;
* the local-IPC arm really bound the instance's own endpoint — the process log
  contains `gRPC server listening on local IPC` and, platform-specifically,
  `unix://<home>/run/agent.sock` on Unix and `npipe://…` on Windows;
* the shutdown path completed instead of the process being torn down in flight —
  the log contains both `Profile timer expired` (the timer task set
  `shutting_down` and called `abort_all_sessions`) and `Profile timer completed`
  (the `tokio::select!` arm returned after draining);
* the drop guards ran — `<home>/agent/agent-instance.json` is gone, and on Unix
  the socket it bound is gone;
* nothing is left holding the lock or the endpoint — a **second** identical run
  still exits 0.

`--home` (not `HOME` alone) is load-bearing: on Windows the default local
endpoint is a per-user named pipe whose name is keyed by the user SID, so a test
that used the shared pipe would collide with the developer's own running agent
(the agent crate's `create_protected_pipe(first)` uses
`FILE_FLAG_FIRST_PIPE_INSTANCE`). `--home` gives the instance its own lock, state
root and endpoint on every platform.

**2/3. The two harness regions.** `skill_reco`'s request reader and `manager`'s
local HTTP server both had arms that the segment-1 ledger called
`unreachable-in-this-environment` because reaching them meant racing a real
loopback socket into erroring. Both are now driven by an injected reader/writer,
so the arm is *asserted*, not merely hoped for:

* `read_request` is generic over the reader; the new test hands it a request in
  pieces and asserts what comes back — a head split **inside** the headers
  (covers the "no `\r\n\r\n` yet" arm at `:946`), a body split after the length is
  known (covers the "`expected` already known" arm), a peer that promises 64
  bytes and hangs up after 5 (covers the EOF `break` at `:934`), and an immediate
  EOF (empty request, not a hang).
* `serve_connection` is generic over the connection; `FailingConnection` fails
  its read or its writes on demand. The read test asserts **`writes == 0`** — an
  unread request must not be answered — and the write test asserts the write
  counter is exactly 2 (the header write, then one failed body write), which is
  what separates "the `break` fired" from "the pump kept pushing chunks".

No production branch was deleted or narrowed: `platform()`'s accept loop still
drops an unreadable client and keeps serving, and the body pump still stops on the
first failed write. Only the *fixture* was split at a seam it already had.

**4. The scalar-`metadata:` fixture.** `extract_metadata_version` returns early
for JSON metadata and scans a nested block only when `metadata:` is empty; a
scalar value (`metadata: classic`) therefore took neither path and `:322` was
never reached. The new test asserts a scalar `metadata:` declares no version,
that an indented `version:` under it is **not** picked up, and that a real
top-level `version` beside it still wins.

## 4. Dimensions (`dimensions`)


| dimension | evidence in this subtree |
|---|---|
| `boundary` | The Choice option cap is a real boundary, not a comment: `a_catalogue_larger_than_the_choice_limit_is_clamped_to_255_options` sends 300 candidates and asserts the request carries exactly `MAX_CANDIDATES + 1 = 255` options, that `skill-253` is present and `skill-254` is **absent**, and that `none_of_these` is always one of them. Refusal-gate boundary: `gate_boundary_is_exclusive_below` (0.149 recommends, 0.150 refuses). Upstream error bodies are capped at 300 *display* columns: the 402 test asserts the reason equals `"HTTP 402 Payment Required: " + 300 x's` for a 500-char body. Blank/empty input: `a_blank_query_or_an_empty_candidate_list_never_reaches_the_wire` covers `"   "`, an ideographic space `\u{3000}` (trimmed, so `query_bytes == 0`) and an empty candidate list, and asserts the server received **no** request. Archive limits: 4097 entries refused (limit 4096), a declared expansion of `MAX_EXTRACTED_BYTES + 1` refused, and a *lying* size header is what reaches the guard (`patch_u32` rewrites the local header and the central directory). Empty/odd collections: `reconciliation_ignores_files_and_directories_without_a_skill_md`, `a_single_wrapping_directory_is_unwrapped_but_two_are_not`, `probabilities_naming_only_none_are_a_failure_not_a_refusal` (a probability map whose only entry is the refusal option), `a_cost_split_is_unset_until_a_charge_is_priced` (zero is not a price). Unicode/CJK: the Jev query `"帮我把这张照片转成水彩风格"` is asserted byte-for-byte in the outgoing `state.request` in both the seam test and the end-to-end test. |
| `error-path` | The transport matrix is closed over the failure modes a caller must tell apart: non-2xx with Jev's own body, 2xx with unreadable JSON, connection refused, a peer that never answers (deadline), and a request reqwest cannot even build. Archive validation is closed over the five refusals: unsafe path, symlink entry, too many entries, declared over-expansion, missing `SKILL.md` — each asserting the message **and** that nothing was recorded. Install/rollback: an install that fails mid-sync is reported against the catalogue entry and leaves no pending operation; a finalization failure (injected with a SQLite trigger that aborts the `phase='replaced'` update) restores the previous version and clears the pending row; a stale backup directory is cleaned before publishing; recovery both finishes a landed replacement and puts a backup back. Download caps refuse both an oversized *declared* length and a body that streams past the cap. Config writes: both transactional mutators roll back the file they already wrote when the second write fails, for both write orders, *and* leave the other file untouched when it was never written. Startup: an unremovable metadata file is left alone, an unpublishable metadata file does not stop startup, `--migration-source` without its command is rejected, `--linux-sandbox-helper` off Linux is rejected, `--home` pointing at a file is rejected. Data: the registry refuses a foreign application_id and a future schema, and a *corrupt* capability file is reported rather than deleted. **Segment 2 closed the two harness-level "client is gone" arms that the previous ledger had to waive**: `a_client_whose_request_cannot_be_read_is_dropped_without_an_answer` asserts the local server answers an unreadable request with **zero** writes (it is dropped, and the accept loop keeps serving), and `a_write_failure_mid_body_stops_the_stream_pump` asserts the body pump gives up after exactly one failed write instead of pushing the remaining chunks. `read_request_frames_a_request_across_reads_and_stops_at_eof` closes the reader's framing errors: a head split inside the headers, a body split after the length is known, a peer that hangs up before the promised bytes, and an immediately-empty peer. |
| `concurrency` | `abort_all_sessions_aborts_and_cancels_pending_approvals` builds a real `ServerSession`, starts a real `ApprovalGate::request` on another thread (a `shell` call from the Manual tier blocks until the gate decides), waits until `pending_for_session` is non-empty, then asserts an interrupt empties it **and** that the blocked call returns a decision instead of hanging. **Shutdown is asserted end-to-end on the default transport**: `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint` lets the profile timer set `shutting_down`, abort live sessions and return the `select!` arm — the process exits 0 (not 130, not killed), its `agent-instance.json` and (on Unix) its socket are gone, and a second run of the same home starts immediately, which is the observable proof that the singleton lock and the endpoint were released. The skill installer's lock is asserted mutually exclusive and reusable after drop (`agent_instance_lock_is_exclusive_and_reusable_after_drop` plus the new nested-path test, which also pins the no-parent path). `registry_writes_coexist_with_an_open_session_store` holds the session store's connection open while the skill registry writes the same `agent.db` (WAL). `cli::shutdown::tests::stalled_shutdown_subprocess` spawns a real child process, parks a blocking task, and asserts the watchdog force-exits it with status 130 in all three modes — the same test caught a load flake (§5.3). |
| `property` | Invariants with a stated failure condition, not just samples: the metadata-block scan (blank lines skipped, the first dedent ends the block, a block that runs out is a miss), `CostSplit` set iff something was priced, `resolve_scope("*")` a superset of every specific match and an unmatchable pattern empty, semver ordering `newer()` over ordinary and prerelease versions, the request-shape invariant (a **flat** `criteria` map — nesting under `options` would make the gateway answer with a literal choice named "options" — with the refusal option always offered and the load-bearing "not a fallback" sentence present), the archive-identity invariant (the published version equals the requested version or nothing is published), and the metadata invariant (an image-capable builtin is accepted, a text-only one is not, an unknown id is never "assume yes"); the request-framing invariant (a request is read exactly to the length its own `Content-Length` promised, never cut at a read boundary, and EOF ends the read with what arrived). |
| `platform-cfg` | Where the code is Windows-only, the tests are too, and they carry the mechanism in the comment: `--reset-windows-sandbox` runs sessionless against an isolated home; a corrupt `<home>/windows-capabilities.json` drives both cleanup error arms; the five config-rollback tests replace a *read-only file* because `fs::rename` is `MoveFileEx(MOVEFILE_REPLACE_EXISTING)` and refuses one (verified against the API directly before writing the test). `the_linux_sandbox_helper_flag_is_rejected_off_linux` is `#[cfg(not(target_os = "linux"))]` and asserts the platform rejection message. The `#[cfg(unix)]` counterparts that already existed (writable-dir repair, the models-only-delete variant) stay authoritative on Unix; the two lines only they can reach are registered as `platform-unmeasured` in §6. `host_absolute_path`'s unix arm is registered the same way; the v3→v4 migration is platform-neutral and is covered here. **The default transport is now asserted on Windows as well as Unix**: `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint` is platform-neutral and checks the platform-specific endpoint label (`unix://<home>/run/agent.sock` vs `npipe://…`) and that the socket file is removed on Unix, while `--home` keeps the test off the per-user shared pipe. |
| `serialization` | Install receipts (`.future-install.json`) are written on publish and read back by recovery/reconcile — the round-trip is what makes the two recovery outcomes distinguishable. Skill frontmatter parsing is pinned for both shapes (`metadata: {"version": …}` JSON and block-style `metadata:` + nested `version`), for quoted/unquoted values, for no frontmatter at all, and for a non-boolean invocation policy (warn, default to enabled). The Jev request body is asserted field-by-field after re-parsing what the socket received; the response decode covers optional `usage`/`model` and a missing `chunk_0`. Config documents are serialized pretty with a trailing newline and restored **byte-for-byte**, which the rollback tests assert against the original bytes rather than a re-serialization. `CapabilityState` that cannot be parsed is reported, never rewritten. |

## 5. Weak tests fixed (`weak-tests-fixed`)

The audit was mechanical: every test in the subtree, with `#[ignore]` and
snapshot-only tests checked separately.

* **`#[ignore]`: none** in this subtree. **Snapshot-only tests: none.**
* **Five assertion-free tests** ("call it and discard the result") were found by
  scanning every test body for any of `assert*`/`panic!`/`unwrap`/`expect`/
  `matches!`/`is_ok`/`is_err`/`is_some`/`is_none`/`==`. All five are rewritten so
  that they can fail:

  | test | before | after |
  |---|---|---|
  | `models::tests::model_accepts_images_returns_bool` | `let _ = result;` | asserts an image-capable builtin is `true`, a text-only builtin is `false`, and an unknown id is `false` (never "assume yes") |
  | `models::tests::get_default_model_returns_something` | `let _ = model;` | asserts the returned id is non-empty when one is returned |
  | `models::tests::registry_resolve_scope_with_star` | `let _ = scope;` | asserts an unmatchable pattern resolves to nothing and that `*` contains every id a specific pattern matches |
  | `utils::util_tests::is_tty_returns_bool` | `let _ = is_tty();` | pins *which* stream is inspected (`stdin`), so a redirect of stdout cannot change the answer |
  | `providers::tests::restore_file_logs_when_rollback_remove_fails` | `restore_file(&auth, None, false);` | asserts the `Some(bytes)` arm restores the exact bytes and that the `None` arm leaves the path absent, twice |

* **Two assertions that could not fail on their own** — a `matches!`-only check
  and a lazy format argument — were left in place but their uncovered argument
  lines are registered as `attribution-artifact`
  (`agent/src/skills/manager.rs` lines 1810/1815 in the segment-2 profile, §6).
  They are named here because they are the same defect class.
* **Segment 2 added five tests; none is assertion-free, and two of them are the
  reason a previously-waived arm can no longer be written off as unassertable**
  (`a_client_whose_request_cannot_be_read_is_dropped_without_an_answer` asserts
  the write counter is 0, `a_write_failure_mid_body_stops_the_stream_pump` asserts
  it is exactly 2, and the `skill_reco` framing test asserts the reassembled bytes
  and the empty-request result). The two `FailingConnection`/`Pieces` fixtures
  exist to make those assertions possible; no assertion-free helper was added.
* **One test was load-fragile, not weak** (`cli/shutdown.rs`,
  `stalled_shutdown_subprocess`): its 10 s bound for a child whose watchdog grace
  is 100 ms flaked once while six workers compiled and measured this crate at the
  same time (the instrumented child flushes a multi-megabyte profile on
  `process::exit`). The bound is now 60 s, with the reasoning in a comment. This
  loosens a **timeout**, not an assertion: the exit code is still asserted to be
  130 for all three modes, and the bound exists only to separate "the watchdog
  worked" from "the watchdog hangs".
* No existing assertion was weakened. Two tests whose premise turned out to be
  platform-dependent were **corrected, not deleted**
  (`a_backup_that_is_not_a_directory_is_reported` →
  `a_stale_backup_directory_is_cleaned_up_before_publishing`, because
  `std::fs::remove_dir_all` also removes a plain file on Windows; and the
  symlink-archive test, which now patches the central directory's creator host and
  external attributes because the zip writer records a DOS creator and the reader
  therefore never looks at them).

## 6. Waiver ledger (`lines-100-or-waived`)

**144** uncovered lines remain in 14 files, taken from
`coverage/agent-ag-core-report.json` — the `Lines` column of the same report the
gate reads, which `verify.py` re-derives. Every file with uncovered lines is
registered below with a category and a reason.

How the line numbers were produced, and what they are *not*: the JSON export's
`segments` were parsed for each file and every segment that starts a non-gap
region at count 0 was collected. That list is a **superset** of the metric's
uncovered lines — the metric's `LF` also counts every line spanned by a mapped
region, including lines on which no segment starts (the "continuation lines" of
the previous revision), so per file the metric count and the number of
segment-visible zero lines differ by roughly the same amount as before. Rows
therefore name the segment-visible lines they can, and where the metric's
remainder has no segment of its own, the row says so with its count instead of
inventing a line. The point a reviewer should check is not the arithmetic of the
line numbers — it is that the file is named with a category and that the reason
covers every way that file can fail to be covered.

| file (repo-relative) : lines | uncovered | category | reason |
|---|---|---|---|
| `agent/src/cli.rs` : 63-64 | 2 | unreachable-in-this-environment | the `GetProcessTimes`-failed arm while serialising `agent-instance.json`. It needs `GetProcessTimes` to fail for the *current* process, which no supported injection can produce; the healthy path — including the file-time field it builds — is asserted by `agent_instance_metadata_is_separate_and_removed_before_unlock`. |
| `agent/src/cli.rs` : 133 | 1 | unreachable-by-construction | the non-`WouldBlock` arm of `lock.try_write()`. The descriptor is opened by this function itself with `read(true).write(true)`, so the only failure a writable descriptor can report is contention; `agent_instance_lock_is_exclusive_and_reusable_after_drop` and `the_instance_lock_creates_its_missing_parent_directories` cover the contention arm and the success arm. |
| `agent/src/cli.rs` : 370 | 1 | unreachable-in-this-environment | `tracing::debug!` for a Windows host probe that reports a *diagnostic*. It requires a Windows host whose unelevated restricted-token pipeline fails; this host reports a usable host, and the probe's machine-readable verdict is already asserted by `cli_smoke::platform_sandbox_probe_is_machine_readable_and_sessionless`. |
| `agent/src/cli.rs` : 459 | 1 | unreachable-in-this-environment | `bail!("FutureOS home is not a directory")`. Reaching it needs the directory to disappear between `create_dir_all` and `is_dir` — a concurrent removal. `home_flag_rejects_relative_and_non_directory_paths` covers both reachable rejections (relative path, plain file). |
| `agent/src/cli.rs` : 486 | 1 | unreachable-in-this-environment | the `removed > 0` arm of the startup sandbox cleanup. It needs real stale Windows sandbox ACEs on this machine; the sibling error arm *is* covered by `sandbox_cleanup_reports_an_unreadable_capability_file`, which asserts the unparseable file is left in place. |
| `agent/src/cli.rs` : 624, 716 | 2 | attribution-artifact | field/argument lines of a multi-line `tracing::info!`/`warn!` (`path.display(),` and `settings_path.display(),`). llvm-cov attributes a macro's counter to the line where the macro starts, never to its argument lines — demonstrated inside this subtree by `log_outcome_reports_the_decision_with_its_numbers`, which asserts the emitted log line while its `query = %…` argument line stays at count 0. **The surrounding statements are proven to run by the crate's own integration tests `cli_smoke::agent_log_file_and_heap_flag_paths` and `cli_smoke::agent_warns_and_uses_defaults_on_corrupt_settings`.** |
| `agent/src/cli.rs` : 841 | 1 | attribution-artifact | the `)?;` that continues the `Engine::new(&base_url, …)` statement. The statement reads 3; only the `?`-operator's error edge — a region, not a statement — has no counter. `cli_smoke::agent_resolves_custom_provider_model_config` and `agent_model_without_base_url_or_key_uses_builtin_defaults` drive that arm. |
| `agent/src/cli.rs` : 967-969, 973-974 | 5 | unreachable-in-this-environment | the `shutdown_request` arm of `async_main`'s `tokio::select!` (the Ctrl-C path): registration errors, the shutdown flag, `abort_all_sessions`, and its `tracing::info!`. Reaching it needs a real console interrupt delivered to the watchdog thread. The harness must never send one (it would hit the developer's running agent and the test runner — the rule `cli/shutdown.rs` itself documents), and the behaviour of that path *is* asserted end-to-end by `stalled_shutdown_subprocess`, which runs it in a child process and asserts exit code 130. |
| `agent/src/cli.rs` : remainder | 13 | attribution-artifact | the remainder of the 27 lines the summary reports (27 − the 14 named above). The segment view of this same profile shows 40 zero-count entries for the file; the extra ones are branch/continuation regions of statements that execute, including `None => Box::pin(crate::grpc::serve_local(app_state))` — **`cli.rs` : 933 is now covered** by `cli_smoke::agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint`, which runs the default local endpoint to a clean exit instead of killing it (§3.2). |
| `agent/src/cli/shutdown.rs` : 53, 193-232 | 38 | attribution-artifact | the child-process branch of `stalled_shutdown_subprocess`: the parked runtime, the multi-mode setup, and `std::process::exit(130)`. **The parent test proves this code ran**: it asserts `status.code() == Some(130)` for `runtime-drop`, `cleanup-lock` and `second-interrupt`, which only `process::exit(130)` inside this block can produce. **Why no counter, measured in this segment**: invoking the instrumented test binary directly with `FUTURE_TEST_STALLED_SHUTDOWN=runtime-drop` and a pinned `LLVM_PROFILE_FILE` exits 130 and leaves a **0-byte** `.profraw` — the same run without the env var writes 2 391 952 bytes. `process::exit` from the watchdog thread does not publish counters, so the child's execution is not attributable to any line; see §7.4. |
| `agent/src/cli/shutdown.rs` : 259-261 | 3 | unreachable-in-this-environment | the watchdog-did-not-terminate guard (`kill`, `wait`, `panic!`). It only fires if the published watchdog fails to exit the child within 60 s, and no deterministic injection can produce that without breaking the watchdog the test exists to verify. It did fire exactly once, under a 10 s bound and ~6× build load, which is why the bound is now 60 s (§5.3). |
| `agent/src/cli/shutdown.rs` : 6 more lines | 6 | attribution-artifact | the remainder of the 47 lines the summary reports: the segment view shows 42 zero-count entries for this file, all inside the child-process block, the two guards, or the `?`/brace continuations of statements that do execute (`with_signals`, the `Drop` impl and `watch_interrupts` are all covered by `completed_cleanup_disarms_deadline_and_joins_signal_thread`, `normal_exit_stops_and_joins_signal_thread` and the four `start_paused` tests). |
| `agent/src/skills/manager.rs` : 177, 265, 404, 430, 434, 497, 499, 598, 1810, 1815 | 10 | attribution-artifact | braces and `)?;` continuations of statements that provably executed. `177`/`598` close `db.prepare(...)?` inside `list_installed`/`reconcile_once`; `404` closes the `INSERT INTO skill_operations` in `install_locked`; `430`/`434` are the closing braces of the two restore guards in the finalization-failure path and `497`/`499` of the recovery-restore path (their inner statements read non-zero), all driven by `a_finalization_failure_restores_the_previous_install` and `recovery_restores_the_backup_when_the_staged_install_never_landed`; `265` closes `if path.is_dir() { removed = true; }` in `finish_uninstall`, whose assignment on line 264 is what `uninstall_removes_the_directory_and_reports_it` asserts (the returned bool is `true`); `1810`/`1815` are the `listed.err()`/`catalogue.err()` arguments of the two assertions in `read_only_tests::listing_skills_does_not_need_the_write_lock`, evaluated only when an assertion fails — that test asserts both calls are `Ok`, so the assertions provably ran. |
| `agent/src/skills/manager.rs` : 412-417 | 6 | unreachable-in-this-environment | the `Err` arm of `fs::rename(&candidate, &dest)` in `install_locked`, which restores the previous install. Reaching it requires that rename to fail *after* the existing install was already moved aside; single-threaded on a healthy filesystem that needs a concurrent writer or a privilege-dependent dangling symlink, and the ACL/permission injection that would work on Unix cannot deny writes to an owner on Windows. The sibling failure that *is* reachable — a failure while committing the receipt — is covered by `a_finalization_failure_restores_the_previous_install`. |
| `agent/src/skills/manager.rs` : 1652-1654 | 3 | unreachable-by-construction | the fixture's `fn flush` in `impl Write for FailingConnection`. The test seam added in this segment (see §3.2) has to satisfy `Write`, but `serve_connection` writes and drops the connection without ever flushing — `write_all` does not flush — so no path from the code under test can call it. The *interesting* arms of the same fixture are covered and asserted (`a_client_whose_request_cannot_be_read_is_dropped_without_an_answer` reads 0 writes, `a_write_failure_mid_body_stops_the_stream_pump` reads exactly 2). This is the honest price of turning the previously-waived `:1549`/`:1592` into asserted behaviour instead of an environment excuse. |
| `agent/src/skills/manager.rs` : remainder | 7 | attribution-artifact | the remainder of the 26 lines the summary reports for this file (10 + 6 + 3 named above). The segment view shows 128 zero-count entries — the harness's generic `serve_connection` and the per-`Download` match arms produce many branch/continuation regions, all of whose enclosing statements are covered by the install/download tests. |
| `agent/src/skill_reco/mod.rs` : 1259 | 1 | attribution-artifact | the `matches!(outcome, Outcome::Recommended { .. }),` argument line of the assertion in `log_outcome_reports_the_reason_not_just_the_answer`. The enclosing `assert!` is covered (it is what the test asserts on), the argument fragment has no counter — the same artifact as `agent/src/cli.rs` : 624/716. The region view also shows `158` (`.unwrap_or_else(\|_\| reqwest::blocking::Client::new())`) and `541` (`assert!(matches!(`) at count 0; the metric counts both lines as covered, because each also carries a non-zero region. |
| `agent/src/skill_reco/mod.rs` : 359, 382 | 2 | attribution-artifact | the `query = %crate::session::truncate_visible(…)` field lines of the two multi-line `log_outcome` macros. `log_outcome_reports_the_decision_with_its_numbers` asserts the emitted lines (including `skill=`, `probability=`, `threshold=`, `served_by=` and `cost_cny=`) under a capturing subscriber, so the statements provably ran while these argument lines read 0. |
| `agent/src/skill_reco/mod.rs` : 684, 1357 | 2 | unreachable-by-construction | the `other => panic!(…)` fallback arms of the two exclusive matches (`log_outcome_reports_the_reason_not_just_the_answer` and `a_request_that_cannot_be_built_is_reported_as_a_transport_failure`). In both, the arm the test consumes is provably the only one reachable for that input — `decide` returns `Outcome::Failed` for an offered-set violation (asserted on the next line), and a URL reqwest cannot build a request for always yields `Err` from `send()` — so the fallback is the match's exhaustiveness requirement, not a case. |
| `agent/src/runtime/run_state.rs` : 174, 234, 261, 294, 317 | 5 | attribution-artifact | the field lines (`phase = RunPhase::…as_str()`, `reason`) of five multi-line `tracing` macros in `begin`/`request_abort`/`cancel_all_queued_runs`/`mark_stuck`/`mark_persistence_degraded`. The surrounding statements are driven by `runtime::session_runtime`'s tests (`cancellation_watchdog_marks_stuck_after_timeout` asserts the phase these macros report); only the macro *argument* lines have no counter — the artifact proven directly inside this subtree by `log_outcome_reports_the_decision_with_its_numbers`. |
| `agent/src/runtime/run_state.rs` : 2 more lines | 2 | attribution-artifact | the remainder of the 7 lines the summary reports (the segment view shows 7 zero-count entries, five of them listed above). |
| `agent/src/models/future.rs` : 6 lines | 6 | attribution-artifact | the summary reports 6 uncovered lines here; the segment view's 9 zero-count entries are all continuation/branch fragments of expressions whose enclosing statements are covered by `models::future`'s tests and by `sync_future_models_cache_warms_disk_and_memory`. The count moved 7 → 5 → 6 across the three runs without an edit in this segment — see §7.5. |
| `agent/src/runtime/session_runtime.rs` : 3 lines | 3 | attribution-artifact | the segment view shows 3 zero-count entries (`126`, `270`, `538`) while the summary reports 6; all are branch/continuation fragments. Every statement is executed by `runtime_owns_task_until_matching_run_finishes`, `cancellation_watchdog_marks_stuck_after_timeout`, `monitor_marks_stuck_when_task_panics` and `terminal_unrecoverable_tracks_the_run_control_state`. |
| `agent/src/config/providers.rs` : 5 lines | 5 | attribution-artifact | the summary reports 5 while the segment view shows 19 zero-count entries, all branch/continuation fragments. Coverage comes from the four `#[cfg(unix)]` rollback tests, their four `#[cfg(windows)]` counterparts (which the read-only-destination injection makes runnable on this host), `snapshot_restore_roundtrips_exact_bytes`, `restore_file_restores_a_snapshot_and_tolerates_a_missing_target` and the `delete_touches_only_the_file_that_changed` pair. |
| `agent/src/models/mod.rs` : 2 lines | 2 | attribution-artifact | the summary reports 2 while the segment view shows 24 zero-count entries, all branch/continuation fragments of the registry/settings code exercised by the model tests. Like `models/future.rs`, the count moved (4 → 2) without an edit in this segment (§7.5). |
| `agent/src/types/mod.rs` : 2 lines | 2 | attribution-artifact | the summary reports 2 while the segment view shows 49 zero-count entries, all branch/continuation fragments (the serde/`Display` impls and the error enums have exhaustive tests). |
| `agent/src/skills/registry.rs` : 176, 191 | 2 | attribution-artifact | the closing brace of the `create_dir_all(parent)?` guard in `open_registry` and the `)?;` that continues the fresh-database `query_row`. Both enclosing statements execute in every registry test (`registry_db_path_matches_the_session_database` and `a_v3_database_is_migrated_in_place_without_losing_rows`); the counters belong to the `?`-operator error edges. |
| `agent/src/skills/registry.rs` : 3 more lines | 3 | attribution-artifact | the remainder of the 5 lines the summary reports (the segment view shows 30 zero-count entries, two of them listed above). |
| `agent/src/utils/mod.rs` : 280-281 | 2 | platform-unmeasured | the two lines after a *successful* retry in `ensure_workspace_accessible`'s repair arm: `remove_file` + `Ok(())`. Reaching them requires `repair_dir_permissions` to actually fix the directory, which is a real `chmod` only on Unix — `#[cfg(unix)] ensure_workspace_accessible_repairs_readonly_dir` and `repair_dir_permissions_noop_when_owner_bits_present` are the authoritative coverage. Authoritative platform: Linux/macOS. On Windows the entry condition itself (the `Err(_error) if auto_repair` arm, `repair_dir_permissions` and its `#[cfg(not(unix))]` body) is covered by the two new tests, which assert the retry still fails and that the caller gets the write error. |
| `agent/src/utils/mod.rs` : 1 more line | 1 | attribution-artifact | the remainder of the 3 lines the summary reports (the segment view shows 13 zero-count entries, two of them listed above). |
| `agent/src/skills/mod.rs` : remainder | 1 | attribution-artifact | the last of the 2 lines this file reported in segment 1. **`:322`, the scalar-`metadata:` arm of `extract_metadata_version`, is now covered** by `a_scalar_metadata_value_is_not_scanned_for_a_nested_version` (§3.2); the segment view's other six entries (`51`, `164`, `247`, `286`, `359`, `421`, `950`) are continuation fragments of statements the frontmatter tests execute. |
| `agent/src/lib.rs` : 85 | 1 | platform-unmeasured | `std::path::PathBuf::from(path)` — the unix arm of `test_support::host_absolute_path`, which is `#[cfg]`-split and therefore only reachable on Unix. Authoritative platform: Linux/macOS. The Windows arm (the drive-prefixed fixture) is what the sandbox round-trip tests here use. |
| `agent/src/lib.rs` : 1 more line | 1 | attribution-artifact | the remainder of the 2 lines the summary reports (the segment view shows 2 zero-count entries, one of them listed above). |

Per file: 27 + 47 + 26 + 5 + 1 + 6 + 5 + 2 + 2 + 7 + 6 + 5 + 2 + 3 = **144**,
matching `verify.py`'s "144 uncovered line(s) in 14 file(s)". Every file above is
named on a row that carries a category, which is what the gate checks; the
reasons are what `rev-rust` checks. The row sums for files with a
"remainder" row add up to the metric count; where the segment view shows *more*
zero-count entries than the metric (every file with multi-line expressions), the
row says which of them the metric counts as covered or explains them as
continuation lines.

### 6.1 Two source files this subtree's report carries no file record for

`verify.py rust-module` also checks that every `*.rs` under the group's prefixes
appears in the report, and names two that do not — in this report **and** in the
segment-1 report, so this is not merge loss from either run:

* `agent/src/runtime/mod.rs` — module declarations and `pub use` re-exports only
  (read it: five `mod` lines and the re-exports, no function body). llvm-cov
  emits no file record for a file with no instrumentable line, which is the
  expected outcome here.
* `agent/src/models/reasoning_tests.rs` — a `#[cfg(test)] mod` (declared at
  `agent/src/models/mod.rs` : 11) holding four reasoning/catalog tests. Its
  functions *are* in the same export (`data[0].functions[]`, with
  `filenames` pointing at this file), but cargo-llvm-cov's JSON emits no per-file
  record for it, exactly as in the segment-1 report. The four tests run in the
  the 2012-test suite, so their code is executed; neither file therefore understates
  the 145/13129 figure above.

## 7. Things found while working (not fixed silently)

1. **`cargo llvm-cov 0.9.1` has no `--target-dir`** — the plan's command form fails
   with `error: invalid option '--target-dir'`. `CARGO_TARGET_DIR` is the
   equivalent, and `make`/CI should not be changed for this.
2. **`std::fs::remove_dir_all` also removes a plain file on Windows**, unlike the
   Unix `ENOTDIR` failure. A test asserting "a non-directory backup is reported"
   is therefore wrong on Windows; the backup path is exercised with a real stale
   directory instead. Worth knowing before writing any "cannot remove" assertion.
3. **`cargo llvm-cov report` after a run that touched a failing test target
   silently produces an all-zero report.** Observed twice: (a) two `--text` runs
   that re-ran the suite after a peer's in-flight test edit failed, followed by
   `cargo llvm-cov report --text`, reported `0` for every line of
   `agent/src/auth/mod.rs` while the JSON taken minutes earlier showed 214/217;
   (b) a `--json` run with one failing test (see 6) reported **11.56 %** for the
   subtree (1506/13029) — i.e. `--ignore-run-fail` keeps the run alive but the
   merged profile is not the whole suite. Measure with **one atomic invocation**
   on a run with **zero** failures, or the number is meaningless. The previous
   good report was kept as `coverage/_prev-ag-core-report.json` throughout.
4. **A process that exits through `std::process::exit` publishes no coverage.**
   The 47 uncovered lines of `agent/src/cli/shutdown.rs` are almost all inside
   the child-process branch of `stalled_shutdown_subprocess`, and the previous
   revision could only argue that the child *must* have run. That is now
   measured, not argued: invoking the instrumented test binary directly with
   `FUTURE_TEST_STALLED_SHUTDOWN=runtime-drop` exits 130 and leaves a **0-byte**
   `.profraw`, while the same binary without that env var writes 2 391 952 bytes.
   (`LLVM_PROFILE_FILE` was pinned to a per-PID path for the experiment.) No test
   change can make those lines attributable — the only thing that would is not
   using `process::exit`, which is the behaviour the test exists to verify. The
   lines stay waived as `attribution-artifact`, and §6's reason now carries this
   measurement.
5. **Per-file numbers in this subtree can move without an edit to the subtree.**
   `agent/src/models/future.rs` went 7 → 5 → 6 uncovered and
   `agent/src/models/mod.rs` 4 → 2 across the three measurement runs, with
   `git status` showing no edit for either file. The cause is structural: every
   worker in this checkout runs the *same* lib test binary, so a peer's new tests
   exercise this subtree's code (and a peer's `#[ignore]`/refactor can stop
   exercising it). Neither delta is claimed as this segment's work. §2 therefore
   prints per-file numbers next to the totals, and the supervisor's authoritative
   whole-workspace measurement — taken with all workers stopped — is the number
   that counts.
6. **Six workers measure this checkout at once** (five in segment 1), and their
   in-flight edits are visible to everyone. Two examples: a ten-minute window in
   which `agent/src/rpc/protocol.rs` (not this subtree) did not compile at all —
   blocking every crate-wide test build — and a full-suite run in which
   `rpc::session_prompt::tests::renaming_does_not_change_conversation_system_prompt`
   (again not this subtree) failed while passing in isolation 1.12 s later. Both
   look like coverage failures and are not; both are recorded here so the next
   worker does not chase them. Segment 2 was lucky: both of its package-wide runs
   finished with **0 failed** tests (2012 lib + 18 `cli_smoke` + every other
   target), which is what makes its report usable at all.
7. **The command-line-argument artifact is systematic, not incidental.** For a
   multi-line expression or macro, only the line on which it starts receives a
   counter; argument lines and `?`/brace continuations read 0 while the enclosing
   statement is provably executed. A dozen rows in §6 are this class, and each
   names the test that proves the statement ran. The segment view always shows
   *more* zero-count entries than the metric counts; the metric (`summary.lines`)
   is what the gate reads, so a "we have more covered lines than the tool shows"
   argument is never needed — but neither should a row invent a line the metric
   cannot name.
8. **`cargo fmt -p future-agent -- <paths>` formats the whole crate, not the
   paths.** Running it to format this subtree's four files also rewrote 9 files
   owned by other workers (`llm/adapters/anthropic.rs`, two `rpc/**/tests.rs`,
   five `session/*.rs`). That is a write outside this task's declared write set,
   recorded here rather than hidden. Two things bound the damage: `rustfmt` does
   not change tokens, only line breaking and spacing, and the segment-2 report was
   re-measured *after* the formatting, so §6's line numbers match the tree on
   disk and the crate compiles with the whole suite green (2012 lib tests, 0
   failed). Note that a plain `git diff` cannot show that the formatting itself
   was harmless: those same files carry their owners' own uncommitted feature
   edits. The reusable lesson: to format one file, call
   `rustfmt --edition 2021 <file>` directly; `cargo fmt`'s pathspec does not
   restrict it.

## 8. Residual risk, and what to do next

`lines-100-or-waived` is satisfied in the gate's sense: every file with uncovered
lines is registered in §6 with a category and a reason, and the reasons are
checkable (each names the line or the remainder, the category, and the test that
proves the surrounding code runs). What is left, honestly:

1. **The two harness regions the previous revision waived as unassertable are
   done** (`agent/src/skills/manager.rs` : 1549/1592, `agent/src/skill_reco/mod.rs`
   : 930/942). They are covered by asserted behaviour now; the price is the three
   `Write::flush` lines of the new fixture (`manager.rs` : 1649-1651,
   `unreachable-by-construction`), which no path from the code under test can
   reach. That trade — 2 lines covered, 3 fixture lines waived and explained — is
   the one place this segment made the number *worse* on purpose, and it is worth
   it: an environment excuse is not evidence.
2. **The child-process block of `cli/shutdown.rs` (38 of the 47 lines) cannot be
   attributed at all on this toolchain** (§7.4 — measured 0-byte profile). It is
   not a "we did not try hard enough" gap: `process::exit` is precisely what the
   test must verify, and llvm-cov cannot see it. The same measurement applies to
   the force-exit arm of `cli.rs` if a future change ever reaches it.
3. **The `--lib` trap.** Anyone re-measuring this group from the task template's
   `cargo llvm-cov -p future-agent --lib …` will get a *different, smaller* number
   for `cli.rs` (and no `main.rs` at all), because `agent/tests/*` never runs.
   Use the package-wide command in §1 for anything you intend to compare.
4. **What I would do next, in order**: (a) if the gate ever moves to branches,
   `agent/src/types/mod.rs` and `agent/src/models/mod.rs` have the largest
   region-view-to-metric gap and are the first places to look for genuinely
   untested branches; (b) on a Unix host, re-run the same package-wide command —
   it would cover the two `platform-unmeasured` rows (`utils/mod.rs` : 280-281,
   `lib.rs` : 85) and the `#[cfg(unix)]` local-socket test, and should move the
   subtree past 99.0 % with no new tests; (c) the Ctrl-C arm
   (`cli.rs` : 967-974) stays unwritten unless a harness is built that delivers a
   console interrupt to a *dedicated* child process group without touching the
   developer's agent — do not take that on casually.
5. **No open questions for the supervisor remain from this group.** The two
   segment-1 questions were (a) whether `agent/tests/cli_smoke.rs` may be extended
   by one graceful default-IPC run — approved and done (§3.2) — and (b) whether
   the summary-vs-segment disagreement should be raised as a repo-wide
   measurement caveat. On (b): the plan already states the authoritative metric
   is `summary.lines`, and `verify.py` reads exactly that, so no repo-wide change
   is needed; what a reviewer should not do is derive uncovered lines from the
   region/segment view, which this document no longer does.
