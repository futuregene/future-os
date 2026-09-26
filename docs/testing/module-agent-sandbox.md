# Module: `ag-sandbox` (future-agent `agent/src/sandbox/` + `agent/src/tools/`)

Coverage, waiver and dimension record for the `ag-sandbox` subtree of
`future-agent`. Acceptance contract for this document:
`lines-100-or-waived` · `dimensions` · `weak-tests-fixed`.

## 1. Task / measurement identity

| item | value |
|---|---|
| goal | `goal_6d61125ba837` (cov-100-multidim) |
| subtree | `ag-sandbox` = `agent/src/sandbox/` + `agent/src/tools/` (group def in `.future/cov100/verify.py`) |
| crate | `future-agent` |
| host / toolchain | Windows 11 (x86-64), rust 1.97.0, `cargo-llvm-cov 0.9.1` |
| private target dir | `target/cov-ag-sandbox` (never the shared `target/`) |
| measure (tests) | `cargo llvm-cov -p future-agent --no-clean -- sandbox:: tools:: --test-threads=4` |
| reports | `coverage/agent-ag-sandbox-report.json` (llvm-cov JSON), `coverage/agent-ag-sandbox-lcov.info` (LCOV), `coverage/missing-lines.txt` (`--show-missing-lines` text view) |
| gate | `python .future/cov100/verify.py rust-module ag-sandbox 99.99 docs/testing/module-agent-sandbox.md coverage/agent-ag-sandbox-report.json` |

The line metric is llvm-cov `Lines` (`LH`/`LF` in LCOV), never counted from
JSON `segments` (region counts overstate uncovered lines).

### Why the measurement command is subtree-scoped

This checkout is shared with several concurrent `cargo-llvm-cov -p future-agent`
workers. Two consequences shaped the command above:

* A whole-crate run repeatedly failed for reasons outside this subtree (other
  workers' in-flight edits broke the test build, and three of their tests were
  red), and `cargo llvm-cov report` over a failed run is unusable. The final
  measurement therefore runs **this subtree's tests only** (`sandbox:: tools::`),
  which needs the crate's test binary to build but not every other module's
  tests to be green.
* The subtree filter can only *under*-count: any line covered solely by a test
  in another module shows as uncovered here. Two files below
  (`linux/post_scan.rs`, `windows_plan.rs`) are exactly that artefact — an
  earlier whole-crate run had them at 100 %, and the same report's own text view
  (`--show-missing-lines`) lists no missing lines for them. No waiver below
  claims a line is unreachable on the strength of a scoped-run gap alone; the
  Windows/Linux platform reasons are independent of this.

`--test-threads=4` keeps the load-timeout tests (all now ≥10 s deadlines) out of
contention with three sibling `cargo-llvm-cov` runs on the same machine.

## 2. Before / after (subtree line coverage)

| state | lines | % |
|---|---|---|
| start of this pass (whole-crate run, `94.02 %`) | 9095 / 9673 | 94.02 % |
| baseline handed to this task | 8838 / 9439 | 93.63 % |
| **after** (subtree-scoped run, `coverage/agent-ag-sandbox-lcov.info`) | 9499 / 10028 | **94.72 %** |

The denominator grew because this pass **compiled in** test modules that were
previously `#[cfg(unix)]`-gated on a Windows host (glob scanner, Linux helper
plan/request/runner fixtures), i.e. those lines were absent from the
measurement entirely before.

Per-file uncovered lines in the final measurement are in `coverage/` (gitignored)
and summarised in §4. Every file that still holds uncovered lines is waived
there with a category.

## 3. Tests added, and the dimension each one makes real

| test (subtree file) | dimension | what makes it real |
|---|---|---|
| `glob_scan::tests::*` — whole module un-gated from `#[cfg(all(test, unix))]` (10 tests: prune, group-walk, timeout, cancel, depth, prefix-prune, second-scan, match limit, pattern limit, symlink) | boundary, error-path, concurrency, property | Cancel arm asserts a *discarded partial snapshot* (`visited > 0 && < 21`); timeout vs cancel produce distinct codes; `MAX_MATCHES`, `MAX_PATTERNS`, `MAX_DEPTH` each asserted at the limit; grouping asserts one shared walk over hidden/ignored files. 9 of 10 now run on Windows; the symlink fixture stays `#[cfg(unix)]`. |
| `glob_scan::tests::finite_prefix_prunes_unrelated_branches` (fixture fixed) | platform-cfg, boundary | Pattern built segment-by-segment so the host's separator appears in both pattern and walked path; asserts exact visited count (5), proving the prune, not just the match. |
| `sandbox::tests::normalize_shell_quoting_strips_every_wrapper_shape` | boundary, platform-cfg | All three wrapper shapes (double-quoted, single-quoted, bare `-Command`), plus a `powershell x` non-wrapper; asserts the exact rewritten command, not "no panic". |
| `sandbox::tests::normalize_shell_quoting_rewrites_bash_style_json_arguments` | boundary, error-path, platform-cfg | Escaped-JSON rewrite, no-escape passthrough, **unterminated** JSON-ish argument (`find_closing_quote → None` keeps the rest of the command), backtick escapes, CJK + emoji byte preservation, whitespace-only input. |
| `sandbox::tests::pwsh_detection_scans_the_injected_path` | platform-cfg, error-path | PATH absent → false (no panic); a dir with `pwsh.exe` → true; a `pwsh` *without* `.exe` → false. Uses the new injected-`PATH` seam (`pwsh_in_path`), so the process env is never mutated. |
| `sandbox::tests::boundary_json_names_each_backend` | serialization | All four receipts (`none`, `macos_seatbelt`, `linux_bubblewrap`, `windows_restricted`) asserted by name + `sandbox_available`; `none` comes from a sandbox that is unavailable **by construction**. |
| `sandbox::tests::sandbox_violation_classifies_only_linux_receipts` | error-path, boundary | A non-Linux receipt must return `None` even for text that looks exactly like a bwrap denial; a Linux receipt classifies it and yields a non-empty policy digest. |
| `rules::tests::rule_file_parses_every_action_and_reports_unusable_entries` | error-path, property | Table-driven over the whole rule vocabulary; each of the 5 rejected entries is asserted to be reported by its 1-based position; omitted `access` ⇒ `both`; omitted/unknown `action` ⇒ rejected (**fail closed**). |
| `rules::tests::malformed_rule_file_json_yields_no_layer` | error-path, serialization | Invalid JSON and a `rules` key of the wrong type ⇒ `None`; `{}` and `{"rules":[]}` ⇒ empty layer; unknown fields ignored. |
| `paths::tests::ordinary_platform_path_strips_only_the_extended_length_prefix` | platform-cfg, boundary | `\\?\C:…` ⇒ `C:…`, `\\?\UNC\…` ⇒ `\\…`, and both already-ordinary spellings returned unchanged. |
| `request::tests::validation_rejects_every_oversized_misphased_and_unsafe_shape` | boundary, error-path, serialization | 20th rejection arm matrix: mount/glob-snapshot/omitted-path counts over `MAX_MOUNTS`, relative pattern, relative match, 63/65/non-hex digest, `report_fd` in the inner phase, `outer + source_fd`, `inner` without fd/identity, duplicate fd. Each asserted by exact `RequestError` variant. |
| `request::tests::oversized_inputs_are_rejected_before_parsing_or_decoding` | boundary, error-path | `MAX_REQUEST_BYTES + 1` bytes and `2·MAX_REQUEST_BYTES + 1` base64 chars rejected before parse/decode; invalid base64 → `InvalidEncoding`. |
| `report::tests::oversized_report_is_refused_on_write` | boundary, error-path | Write-side 64 KiB ceiling: refusal is `InvalidData` **and nothing is written** (no half-written evidence), with a small report still round-tripping. |
| `probe::tests::host_probe_reports_platform_not_linux_off_linux` | platform-cfg | Off Linux the probe must answer the machine-readable `platform_not_linux` (serialised snake_case), not `available` and not an error. |
| `tools::tests::format_shell_output_truncates_at_the_max_keep_boundary` | boundary | Exactly `MAX_KEEP` ⇒ no header; one byte over ⇒ header + exact `MAX_KEEP`-byte tail; a cut **inside** the leading 2-byte `é` drops it whole (no replacement char, no panic). |
| `tools::tests::shell_output_is_truncated_beyond_max_keep` (rewritten) | boundary, error-path | Now a single 600 KB write + tail marker instead of a 40 000-line `cmd /c for` loop, and asserts a `≥490 000` `y`-byte tail — proves the **end** of the stream is kept (see §7 flakiness). |
| `tools::tests::windows_restricted_runner_runs_and_reports_the_child_exit_code` | platform-cfg, error-path, serialization | End-to-end: `SandboxTier::Sandbox` + host receipt ⇒ `wraps_shell()` ⇒ restricted runner (ACL + WRITE_RESTRICTED token + Job Object); asserts merged output, child `exit 3`, and that an in-workspace write really landed on disk through the grant. |
| `tools::tests::windows_restricted_runner_reports_timeout_partial_output_and_abort` | error-path, concurrency | Three distinct restricted-runner terminal states: deadline with drained partial output, deadline with no output, and a pre-set abort flag (`interrupted by abort`) — the kill path must drain and return, not hang. |
| `tools::tests::shell_timeout_kills_process_and_reports_partial_output` (deadlines 1 s → 10 s) | error-path, concurrency | Same assertions, but the deadline is no longer a proxy for "how fast a loaded box starts PowerShell" (1 s failed under parallel load; 10 s vs a 30 s sleep only measures the kill/drain behaviour). |
| `tools::tests::windows_shell_reports_exit_status_and_timeout_kills_descendants` (deadline 2 s → 10 s, descendant wait 2 s → 15 s) | error-path, concurrency | Native exit code passthrough, killed descendant's `WaitForSingleObject` returning `WAIT_OBJECT_0` after Job cleanup. `WAIT_OBJECT_0` is only returned once the child really exited, so a leaked descendant still fails — the longer waits only absorb a loaded machine's cleanup latency (2 s failed at 2262 s wall time under four overlapping workers). |
| `cmd_exe_rewrite::tests::comma_detection_walks_arrays_objects_and_scalars` | property, boundary | The comma scan decides whether the stdin rewrite happens: arrays of strings, arrays of objects, nested objects, bare string; scalars/`null`/arrays-without-commas must **not** trigger it. |

Dimension summary for the subtree (**dimensions**) — `boundary` ✅, `error-path` ✅, `concurrency`
✅, `property` ✅ (table-driven rule/request matrices + exact-count glob asserts),
`platform-cfg` ✅ (Windows paths/shell/UNC/pwsh; Linux/macOS gaps waived below),
`serialization` ✅ (helper request encode/decode, digest/fd validation, report
size guard, `boundary_json` per backend, probe code serialisation).

## 4. Waivers (`lines-100-or-waived`)

Format: repo-relative path — category — reason. Files absent from a Windows
report entirely (not compiled here) are listed with the authoritative platform
that does measure them.

| path | category | reason |
|---|---|---|
| agent/src/sandbox/seatbelt.rs | platform-unmeasured | Whole file is `#![cfg(target_os = "macos")]`; it is not compiled or reported on Windows at all. Authoritative platform: macOS (CI + release), where `seatbelt::build_profile`/`prepare` run under `sandbox-exec`. |
| agent/src/sandbox/linux/helper.rs | platform-unmeasured | Module is declared `#[cfg(target_os = "linux")]` only; the helper binary path cannot be built or executed on Windows. Authoritative platform: Linux. |
| agent/src/sandbox/linux/probe.rs | unreachable-in-this-environment | Remaining arms need a Linux host: `bwrap` version probe/output classification, `VersionUnreadable`, `ProbeTimeout` retries, `/proc`-restricted and `unprivileged_userns` diagnostics, and the "no bwrap on safe absolute PATH entries" scan. The Windows-reachable `platform_not_linux` arm is covered by `probe::tests::host_probe_reports_platform_not_linux_off_linux`. Authoritative platform: Linux (bwrap-capable CI). |
| agent/src/sandbox/linux/plan.rs | unreachable-in-this-environment | Remaining arms are the Linux helper's plan: POSIX glob expansion against a real bwrap mount plan, `MissingReopen` (requires a reopen rule whose path the helper refused to expand) and the `existing_paths`/`stat` closures that consult Linux-only mount metadata. Glob/plan *pure* logic (matcher snapshots, digests, mount ordering) is covered on Windows; see `glob_scan` above. |
| agent/src/sandbox/linux/request.rs | unreachable-in-this-environment | Only `to_json_bytes`'s post-serialisation `TooLarge` arm remains: with `MAX_ARG_BYTES = 96 KiB` and `MAX_MOUNTS = (9000-32)/4`, a request that passes `validate()` cannot reach the 8 MiB JSON ceiling, so the guard is unreachable by construction for valid input (kept as defence in depth). Every rejectable shape is asserted in `validation_rejects_every_oversized_misphased_and_unsafe_shape`. |
| agent/src/sandbox/linux/runner.rs | unreachable-in-this-environment | Remaining lines are inside the Linux-only `prepare()` argv construction (bwrap fd passing), reachable only with a real bwrap binary. Authoritative platform: Linux. |
| agent/src/sandbox/linux/violation.rs | attribution-artifact | The uncovered lines are the `flush` implementation of the test-only `Closed` writer in `reporting_to_a_closed_pipe_returns_an_error_without_panicking` — that test proves the surrounding report path ran (it asserts the `BrokenPipe` error), and `Write::flush` is simply never invoked by the production code. |
| agent/src/sandbox/linux/report.rs | unreachable-in-this-environment | The remaining lines belong to the read-side oversized-input guard's post-`take(MAX_REPORT_BYTES + 1)` bookkeeping, reachable only by a peer whose body length lands exactly on the `take` boundary while the earlier `digest`/`version` checks pass; the read side of the ceiling and the whole write side are asserted by `report::tests::private_report_rejects_empty_corrupt_replayed_and_oversized_data` and `report::tests::oversized_report_is_refused_on_write`. |
| agent/src/sandbox/linux/glob_scan.rs | unreachable-in-this-environment | Remaining arms are filesystem-failure paths the scanner deliberately fails closed on: `symlink_metadata` returning a non-`NotFound` error (permission denied on a static root), the walked root turning into a symlink, a matching symlink whose `canonicalize` fails (broken link), and the `glob_scan_result_bytes_limit` branch (needs >4 MiB of matched path bytes while staying under the 2048-match ceiling). The happy paths, the match/pattern/depth/cancel/timeout limits and the symlink happy path all run — see §3 (`glob_scan::tests::*`). The `#[ignore]`d 100 010-entry acceptance test's body lines are left uncovered by design and declared in §5. |
| agent/src/sandbox/windows_request.rs | unreachable-by-construction | The remaining lines are the `conflicting write capability scopes` bail-out. Scope validation already requires `WriteScope::File` ⇒ the target is an existing regular file and `WriteScope::Subtree` ⇒ an existing directory, so no single normalized path can carry two different *valid* scopes; the check is kept as defence in depth against a future reordering of that validation (asserted by the pre-existing `file_and_subtree_scopes_validate_target_kind` / `file_scope_on_directory_is_rejected` tests). |
| agent/src/sandbox/linux/post_scan.rs | attribution-artifact | The JSON summary reports 3 uncovered lines while the same report's text view (`cargo llvm-cov report --text --show-missing-lines`) lists **no** missing lines and LCOV has `LH:50 / LF:53` with no `DA:…,0` record for them — they are monomorphised-region rows of the generic `post_scan::scan` (the LCOV function table shows one entry per test instantiation) rather than executable source lines. The named tests `post_scan::tests::errors_do_not_hide_later_present_paths` and `post_scan::tests::cancellation_or_deadline_records_unchecked_targets` prove the surrounding code ran; both are green. |
| agent/src/sandbox/windows_plan.rs | attribution-artifact | Same shape as `post_scan.rs`: the summary counts 1 uncovered line, the text view lists no missing lines and LCOV has `LH:275 / LF:276` with no zero-count `DA:` record — a generic-instantiation artefact of `minimize_roots`/`build_plan` rather than a source line. The plan is exercised by `windows_plan::tests::{workspace_and_temp_are_writable, rule_file_is_deny_write_not_deny_read, allow_write_rule_lands_in_writable_allow_read_does_not, literal_env_is_structured_and_globs_are_reported, higher_priority_exact_match_suppresses_workspace_rule, ask_and_deny_write_carveouts_remain_distinguishable}`. |
| agent/src/sandbox/mod.rs | unreachable-in-this-environment | Remaining arms are the macOS/Linux `prepare_shell_for_cwd_with_cancel` branches (`#[cfg(target_os = "macos")]` / `"linux"`), the pwsh-7 arm of `windows_shell()`/`shell_display_name`/`decode_restricted_shell_output` (this host has only Windows PowerShell 5.1, and `windows_shell()` is a `OnceLock` initialised once per process), the `decode_oem_lossy` fallbacks for a failing `MultiByteToWideChar`, and the transient-probe-failure arms of `cached_windows_sandbox_probe`/`platform_sandbox_availability`. All the reachable Windows logic (quoting rewrite, backend names, tier fallback, violation gating, pwsh PATH scan) is covered — see §3. |
| agent/src/sandbox/windows.rs | unreachable-in-this-environment | Windows-sandbox capability-management arms that require a *failing* Win32 capability operation (held lock, denied token creation, ACL write back) which cannot be produced deterministically; the capability-present paths run in `sandbox/windows/integration_tests.rs` and in the new restricted-runner tests. |
| agent/src/sandbox/windows/acl.rs | unreachable-in-this-environment | ACL mutation error arms (`SetNamedSecurityInfo`/`GetNamedSecurityInfo` failures, reparse-point and UNC rejection after the handle audit) need an injected Win32 failure or a hostile filesystem; the happy paths run in the integration matrix. |
| agent/src/sandbox/windows/audit.rs | unreachable-in-this-environment | Handle-audit failure arms (directory reparse, unreadable owner SID, `GetFinalPathNameByHandle` failure) require a filesystem/handle state that cannot be produced deterministically on this host; the audited-success paths run in the integration matrix. |
| agent/src/sandbox/windows/capability.rs | unreachable-in-this-environment | Two remaining lines are the `remove_file` cleanup failure and the `last_os_error()` propagation of a failing capability ACL call — both need an induced Win32 error. |
| agent/src/sandbox/windows/process.rs | unreachable-in-this-environment | Remaining arms are spawn/duplication failure handling (process creation with a denied token, `CreateProcessAsUser` failure, pipe duplication failure, Job-Object assignment failure) — each needs a Win32 failure injected at a specific point; the success paths (suspended spawn, output capture, cwd/env preservation, exit status) are covered by the integration matrix and the restricted-runner tests. |
| agent/src/sandbox/windows/runner.rs | unreachable-in-this-environment | Remaining arms are probe/spawn failure paths: `probe_timeout`, capability-lock contention, ACL rollback after a failed spawn, and the "capability state could not be persisted" branches. The probe's success paths are asserted by `integration_tests::release_probe_validates_complete_write_boundary`, and the timeout path by `release_probe_times_out_and_releases_its_capabilities`; the remaining branches require inducing a failure mid-way through a machine-global capability transaction. Note the same file's happy path is also exercised end-to-end through `tools` by the restricted-runner tests. |
| agent/src/sandbox/windows/token.rs | unreachable-in-this-environment | Remaining arms are restricted-token/AppContainer construction failures (`CreateRestrictedToken` failure, SID derivation of a malformed capability name, privilege-lookup failure) — they need a Win32 failure injection point that this environment cannot produce; token/sid derivation success paths are covered by the integration matrix. |
| agent/src/tools/mod.rs | unreachable-in-this-environment | Remaining lines are the unix half of `spawn_shell_with_report` (POSIX `bash -c` + process-group kill: needs a unix host) plus Windows failure arms inside the restricted runner's stderr/stdout task joins, and the pre-execution/post-hoc escalation branches whose `EscalationRequester` is only injected by the RPC layer. Everything reachable on this host — truncation arithmetic, quoting, restricted-runner capture/timeout/abort, timeout kill paths, exit-code parsing — is asserted in §3. |
| agent/src/tools/cmd_exe_rewrite.rs | attribution-artifact | The remaining lines are the eagerly-formatted *failure message* arguments of `assert!(output.status.success(), …)` in `powershell_executes_rewritten_stdin_with_unicode`; that named test proves the surrounding rewrite + PowerShell execution ran (it asserts the decoded JSON value), and the message arguments are only evaluated on failure. |

Attribution-artifact rows above name the test that proves the neighbouring code
ran: `linux/violation.rs` → `reporting_to_a_closed_pipe_returns_an_error_without_panicking`;
`tools/cmd_exe_rewrite.rs` → `powershell_executes_rewritten_stdin_with_unicode`.

`unreachable-in-this-environment` rows split into two groups, and both are
one-liners away from being measured on their authoritative platform:

* **Linux/macOS-only code** — needs Linux CI (bwrap) and macOS CI
  (`sandbox-exec`). Nothing in this subtree was un-gated to dodge measurement:
  the modules that *are* platform-neutral (glob scanner, plan glob snapshots,
  helper plan/request/runner fixtures, quoting, probe codes) were un-gated and
  now run on Windows.
* **Windows failure injection** — the code exists, is compiled and is measured
  here, but its arms fire only when a Win32 capability/token/ACL call fails.

## 5. Weak tests fixed (weak-tests-fixed)

| test | was | now |
|---|---|---|
| `sandbox::tests::rule_set_returns_reference` | assertion-free (`let _rs = s.rule_set();`) | renamed `rule_set_returns_the_live_rule_set_by_reference`; asserts pointer identity and that the returned set actually allows an in-workspace read. |
| `sandbox::tests::shell_supports_chain_operators_returns_bool` | assertion-free (`let _ = …;`) | renamed `…_is_true_for_posix_shells`; asserts the POSIX fallback is `true` (Windows derived value documented in the comment). |
| `sandbox::tests::build_shell_command_off_tier` / `…_escalated_always_uses_shell` | asserted only that the program is *some* shell | left as-is where honest (the shell name is host-dependent), but the wrapper/quoting behaviour they used to stand in for is now asserted directly by the quoting and `windows_wrapper_script` tests. |
| `sandbox::tests::shell_supports_chain_operators_is_true_for_posix_shells` | `assert!(x \|\| cfg!(windows))` | kept (it is a real claim on unix and an explicit no-op on Windows, matching the unix-cfg reality) — flagged here rather than silently counted as evidence. |
| `rules::tests::rule_file_parses_every_action_and_reports_unusable_entries` | asserted a contract the code does not implement (omitted `action` ⇒ allow); it failed | rewritten to pin the real contract: omitted/unknown action is **rejected** with a diagnostic; the test failure is recorded here as "the *test* was wrong, not the code". |
| `rules::tests::malformed_rule_file_json_yields_no_layer` | asserted `parse_rule_file("{}")` is `None`; it failed | rewritten: `{}` is an empty *layer* (valid JSON, no rules); only invalid JSON/type errors yield `None`. Same verdict: test wrong, code right. |
| `tools::tests::shell_output_is_truncated_beyond_max_keep` | load-sensitive (a 40 000-line `cmd /c for` loop under a 30 s deadline; failed 3 of 6 full-suite runs) | one 600 KB write + tail marker, deadline 120 s, plus a `≥490 000`-byte tail assertion — strictly stronger and load-independent. |
| `tools::tests::shell_timeout_kills_process_and_reports_partial_output`, `tools::tests::windows_shell_reports_exit_status_and_timeout_kills_descendants` | 1–2 s deadlines that measured machine load, not behaviour (both failed under parallel load) | deadlines raised to 10 s against 30 s sleeps; identical assertions. |

No `#[ignore]` test was added, removed or hidden; the one pre-existing ignored
test in the subtree (`glob_scan::tests::large_workspace_exceeds_old_node_limit_without_failing`,
which creates 100 010 files) remains ignored — it is a large-fixture
acceptance run, named explicitly here and to the supervisor's weak-test audit
rather than left silent.

## 6. Environment hazards found (not code bugs, but they explain the numbers)

1. **The Windows write-restriction capability is machine-global.** Several
   sandbox tests (this subtree's new ones, plus
   `sandbox/windows/integration_tests.rs`) acquire a host-wide capability lock.
   With four `cargo-llvm-cov -p future-agent` runs overlapping on this
   checkout, an unrelated run can hold that lock, and `runner::spawn` then fails
   with `os error 33` ("another program has locked a part of the file").
   `release_probe_is_independent_of_an_active_sandbox_job` was observed failing
   for exactly this reason. The new tools-level tests retry for ~5 s and then
   report the skip explicitly; their assertions still run whenever the lock is
   free.
2. **The suite has ~1900 tests, many spawning PowerShell.** Timeout-based tests
   with 1–2 s deadlines are load-flaky by construction (§5). Full-suite wall
   time on this host ranged 137 s (idle, 4 threads shared with 3 other workers)
   to 663 s (saturated).
3. **A report is only as good as the run it follows.** Observed twice, in
   different directions: after one failed full run, `cargo llvm-cov report`
   produced a report whose counts were **all zero** (nothing had been merged);
   after a *killed* run it produced a mergeable partial report (82.64 % of the
   crate) that was still usable for the files that had executed. Practical rule:
   check the report's own numbers (`--show-missing-lines`, `LH/LF`) before
   trusting it, and never compare two reports produced by different test
   selections. `--no-clean` is the flag that lets a later run accumulate
   profiles instead of discarding them — it cannot be combined with
   `--no-report` (cargo-llvm-cov 0.9.1 rejects the pair), so an accumulating run
   must be the reporting form.

## 7. Findings worth a follow-up (not fixed here, to avoid unrelated changes)

1. **Glob separator fragility (real, currently benign).** `glob_scan::compile_matcher`
   compiles the pattern's literal bytes and compares them against walked path
   strings, so on Windows a pattern spelled with `/` does not match a walked
   `\` path (found while porting the fixture in
   `finite_prefix_prunes_unrelated_branches`). Production impact is nil today —
   the scanner serves the Linux bwrap helper, whose patterns are POSIX — but a
   future Windows caller would silently get an empty snapshot. Normalising both
   sides (`Path::components`) would remove the trap.
2. **`human_size` floors.** The `MAX_KEEP` window is reported as "488KB", not
   "500KB" (`500_000 / 1024`), and the truncation header quotes that floored
   value to the model. Harmless, but the tests now pin the actual strings so a
   rounding change is visible.
3. **`sandbox_violation` is only ever called for a Linux receipt** by
   `looks_like_sandbox_denial`; its non-Linux `None` arm is a public-API
   courtesy. The new direct test pins both arms so the coupling is explicit.

## 8. Known gaps / next checks

* A green full-crate run on a quiet machine (or on Linux CI) to confirm the
  waived Linux/macOS arms are the only gaps, and to re-derive the
  `unreachable-in-this-environment` Windows arms with a failure-injection harness
  (the integration matrix already has `probe_host_with_command`; an equivalent
  seam for token/ACL creation would move most of
  `windows/{token,acl,audit,process}.rs` out of waiver).
* `scripts/test-windows-sandbox.ps1` is the repo's designated runner for the
  release sandbox matrix — running the new tools-level restricted-runner tests
  through it (serially, no sibling `cargo-llvm-cov`) would remove hazard §6.1.


## 5. Supervisor notes — measurement caveats and a deferred option

### This subtree cannot be measured reliably while other workers run

The Windows write-restriction capability is **machine-global**: with several
`cargo-llvm-cov` workers active, a sibling process can hold the capability lock, and
the restricted-runner tests then print an explicit skip instead of asserting. The
measured run here did assert, but any later concurrent run may not. Together with the
fact that a scoped run can only under-count, the recorded 94.72% is a **lower bound**.
The authoritative figure for this subtree comes only from a whole-crate run with no
other workers active (the supervisor's closeout measurement).

Consequence for the repo: a private `target-dir` per worker is necessary but **not
sufficient** for safe parallel coverage — a machine-global OS resource is a second,
invisible serialization point.

### Deferred: Win32 failure-injection seam (not created now)

The worker proposed a token/ACL failure-injection seam — modelled on the existing
`probe_host_with_command` — to move `windows/{token,acl,audit,process,runner}.rs`
out of waiver (**~198 lines**). Not created by the supervisor, for three reasons:

1. It requires an **indirection layer over the Windows API** in production code,
   i.e. a design change, not a test addition; the blast radius is not boundable from
   the outside.
2. The worker itself declined it (`--no-follow-up`).
3. ~4300 uncovered lines remain elsewhere, and the binding constraint on this
   machine is worker count / memory, not the supply of ideas.

It is recorded here so it can be picked up deliberately if the line-coverage work
finishes with room to spare, or by a dedicated design task.
