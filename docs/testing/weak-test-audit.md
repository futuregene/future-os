# Weak-test audit — assertion-free tests

**Acceptance contract: `weak-test-audit`.** Declared gate:
`python .future/cov100/verify.py weak` → **PASS (exit 0)** — 58 assertion-free
entries reported, every one adjudicated below. Sibling artifact:
`disabled-test-audit` (gate `python .future/cov100/verify.py skipped`).

Audit of every test the goal's scanner reports as containing **no assertion**.
The scanner is a *starting point, not a verdict*: this document opens each
offender and says what it actually does, so a reviewer (`rev-rust`) can check
each claim against the file:line it names.

Author: cov100 audit task (owner = `rev-rust` reviewer). No source or test file
was modified to produce this document — it is a fact-finding artifact only.

## How to reproduce the worklist

```bash
python .future/cov100/verify.py weak
```
(Before this file existed: `FAIL: 70 assertion-free test(s) not covered by
docs/testing/weak-test-audit.md`, followed by the 70 `[UNAUDITED]` entries.)

The scanner's own heuristics, for context: it looks for `#[test]` /
`#[tokio::test]`, brace-balances the body (string-literal aware), accepts
`should_panic`, and treats these as assertions:
`assert|debug_assert|panic!|expect_err|unwrap_err|unwrap|expect|matches!|is_err|is_ok`
plus any call matching `\w*(check|assert|verify|expect)\w*\s*\(`.
That allowance list is why the false positives below exist.

## Summary — 70 reported entries, adjudicated

| classification | meaning | count |
|---|---|---|
| **真弱 (real weak)** | the test asserts nothing observable; a concrete assertion must be added | **13** |
| **假阳性 (false positive)** | an assertion *is* made, in a helper/macro the scanner cannot see | **43** |
| **合理豁免 (justified)** | no assertion is possible; the test's only claim is a real one (no panic / no hang / no-op by construction) | **14** |
| | **total** | **70** |

**Fix status (updated 2026-09-26, by the weak-test task).** **13 of the 13 真弱
rows now carry real assertions**: 9 in the first pass (`agent/src/models/mod.rs`
×3, `agent/src/sandbox/mod.rs` ×4, `agent/src/tools/mod.rs:2953`,
`channels/src/providers/cli.rs:342`), 3 more in
`agent/src/rpc/prompt_helpers.rs` once `tools::is_approved_outside_path` was
widened to `pub(crate)` (visibility only — the decision is recorded below), and
the last one, `tui/src/components/chat_area.rs:2598`, in a follow-up pass once
the worker editing `tui/` had finished (that row was deliberately held back).
Fixed rows read `真弱 → 已修` in the tables; `docs/testing/weak-test-fixes.md`
is the per-row assertion ledger. (That ledger still lists
`tui/src/components/chat_area.rs:2598` under **Skipped** — it was written before
this follow-up pass and is outside that pass's write scope; the assertion that
closes the row is described in the table below, and the ledger section needs a
one-line update by its owner.)

**Correction (independent verification pass, 2026-09-26).** The twelve fixed
rows are listed above, but **nine of their table cells still read `真弱`** — the
cells were written before the fixes landed and were never updated. Each of the
nine is now verified *absent* from the scanner's worklist (`python
.future/cov100/verify.py weak` reports 58 entries and not one of these names is
among them, which is only possible if the test now contains an assertion):
`agent/src/models/mod.rs:1771`, `:1844`, `:1934`;
`agent/src/sandbox/mod.rs:1458`, `:2049`, `:2064`, `:2080`;
`agent/src/tools/mod.rs:2953`; `channels/src/providers/cli.rs:342`. The cells
and (where they had drifted) the line numbers below are corrected in this pass;
the judgement prose in those nine rows still describes the **pre-fix** body —
`docs/testing/weak-test-fixes.md` is the ledger of the assertion that replaced
it. The one row
this pass could not touch, `tui/src/components/chat_area.rs:2598`, was left to
the worker editing that module and is **closed** in the follow-up pass below —
and closing it exposed that the row's own fix suggestion (also flagged `rev`)
was measurably wrong. None of the 13 is reported by
`python .future/cov100/verify.py weak` any more.

The three `agent/src/rpc/prompt_helpers.rs` rows were the one place the audit's
suggested fix needed a production change. It was made deliberately: the
alternative (re-classifying them as 合理豁免) would have left three tests that
assert nothing, which is what this goal exists to remove.

### The audit's own claims, machine-checked (2026-09-26)

Two claims above are load-bearing — that the nine `真弱 → 已修` rows really do
assert now, and that the deleted assertions were replaced with real ones — so
both were re-derived by script rather than restated.

**A. The nine re-labelled rows: every body contains an assertion.** Each test's
body was brace-matched and scanned for `assert!/assert_eq!/assert_ne!/panic!/matches!/expect_err/unwrap_err`:

```
OK  model_accepts_images_returns_bool                asserts=5
OK  get_default_model_returns_something              asserts=3
OK  registry_resolve_scope_with_star                 asserts=3
OK  legacy_bash_probe_runs_real_bash_version_check   asserts=3
OK  hydrate_skips_when_shell_spawn_fails             asserts=1
OK  hydrate_ignores_dump_without_marker              asserts=1
OK  hydrate_times_out_and_kills_hung_shell           asserts=2
OK  approve_outside_path_adds_to_approved_list       asserts=6
OK  the_pump_stops_when_the_consumer_goes_away       asserts=3
-> rows with zero assertions: 0
```

That is consistent with the independent result that none of the nine appears in
the `verify.py weak` worklist.

**B. The documented replacements exist.** Fourteen literal probes drawn from the
"what replaced it" column of the weakening table — including the table-driven
thinking budgets `("xhigh", 24000)` / `("bogus", 0)`, the `is_empty()`
replacement in `replan_obligation.rs`, the `#[cfg(unix)]` twin in
`cli/src/utils/files.rs`, `location(&seen[0])` and the two `pump_until_*`
helpers in `tui/src/app.rs`, the two `remote.rs` message assertions, and
`Status::Running` in `terminal/manager.rs` — **all 14 resolve**. No row in that
table names an assertion that is not in the tree.

### The three false-positive shapes, with evidence

1. **`cli_ok(...)` — the loop CLI harness.** `orchestration/loop/tests/common/mod.rs:61`
   ```rust
   pub fn cli_ok(args: &[&str]) {
       cli(args).unwrap_or_else(|e| panic!("cli {args:?} should succeed: {e}"));
   }
   ```
   Every loop console test below runs real commands through this helper, which
   **panics on a non-zero exit**, i.e. it asserts the command succeeded. The
   scanner's allowance regex does not match the name `cli_ok`. (28 entries.)
2. **`let _ = (!skip_if_root()).then(<fn item>);` — delegation to a `*_body`
   function.** Used where a chmod-based failure injection does not work for
   root; the comment in the source explains the branchless form. The assertions
   live inside the `*_body` function. (7 entries: 5 in
   `agent/src/config/providers.rs`, 1 in `agent/src/sandbox/windows/capability.rs`,
   1 in `tui/src/rpc/grpc_client.rs` — that last one via `wait_conn`.)
3. **Scanner artifact — a literal `#[tokio::test]` inside a doc comment.**
   `cli/src/test_env.rs:13` mentions `` `#[tokio::test]` `` in the module prose;
   the scanner matched that text and then attributed the *next* `fn` in the file
   (`lock_env`, line 16) to it. The file's real test,
   `wait_for_exhaustion_returns_false` (`cli/src/test_env.rs:120`), asserts on
   line 121. (1 entry.)

---

## Entries

`class` ∈ {真弱, 假阳性, 合理豁免}. `rev` = "call for reviewer to re-check"
(marked where the judgement, not the fact, is the debatable part).

### orchestration/loop

| file:line | test | class | judgement (evidence) |
|---|---|---|---|
| `orchestration/loop/tests/agent_run_drive.rs:1286` | `models_text_and_json` | 假阳性 | every effect asserted through `cli_ok` (`tests/common/mod.rs:61`); `models --format json` must exit 0. |
| `orchestration/loop/tests/agent_run_drive.rs:1296` | `models_sparse_payload_defaults` | 假阳性 | same `cli_ok`; a sparse model payload must not fail the command. |
| `orchestration/loop/tests/console_drive_cov100.rs:34` | `runs_compact_cutoff_without_runs_succeeds` | 假阳性 | `cli_ok(&["runs","compact",...])` — the no-op report must exit 0. |
| `orchestration/loop/tests/console_drive_cov100.rs:177` | `lease_reclaim_same_agent_is_idempotent` | 假阳性 | two `cli_ok` claim calls; both must succeed. |
| `orchestration/loop/tests/console_drive_cov100b.rs:115` | `worker_list_scans_live_sessions` | 假阳性 | `cli_ok` text + `--json` list. |
| `orchestration/loop/tests/console_drive_cov100c.rs:39` | `todo_complete_is_idempotent` | 假阳性 | `cli_ok` twice; second completion must not error. |
| `orchestration/loop/tests/console_drive_cov100c.rs:196` | `run_resumes_alive_session_via_flag` | 假阳性 | `cli_ok` on `run --resume-session`. |
| `orchestration/loop/tests/console_drive_cov100c.rs:221` | `run_falls_back_to_fresh_for_dead_session` | 假阳性 | `cli_ok` on the fresh-session fallback. |
| `orchestration/loop/tests/console_drive_cov100c.rs:293` | `frontier_show_terminal_yes` | 假阳性 | `cli_ok` after closing the last todo. |
| `orchestration/loop/tests/console_drive_cov100c.rs:354` | `worker_list_shows_running_when_streaming` | 假阳性 | `cli_ok(&["worker","list",...])`. |
| `orchestration/loop/tests/console_drive_cov100c.rs:372` | `worker_stop_abort_and_delete_failures` | 假阳性 | `cli_ok` on the *degraded* path (abort/delete fail) — the command must still succeed. |
| `orchestration/loop/tests/console_final_console.rs:13` | `agent_list_json_and_workspace_conflict` | 假阳性 | 5× `cli_ok`. |
| `orchestration/loop/tests/console_final_console.rs:59` | `authority_sets_write_scope_and_approval_gates` | 假阳性 | `cli_ok` with both flags. |
| `orchestration/loop/tests/console_final_console.rs:76` | `scheduler_ack_with_every_flag` | 假阳性 | `cli_ok` with every flag. |
| `orchestration/loop/tests/console_final_console.rs:98` | `scheduler_tick_show_and_liveness` | 假阳性 | 7× `cli_ok`. |
| `orchestration/loop/tests/console_final_console.rs:172` | `scheduler_record_host_failure_bootstraps_state` | 假阳性 | `cli_ok`. |
| `orchestration/loop/tests/console_final_console.rs:196` | `heartbeat_with_agent_id` | 假阳性 | `cli_ok(&["heartbeat-prompt",...])`. |
| `orchestration/loop/tests/console_final_console.rs:203` | `attention_and_inbox_json` | 假阳性 | 6× `cli_ok`. |
| `orchestration/loop/tests/console_final_console.rs:251` | `commands_json_and_canary_bare_smoke` | 假阳性 | 7× `cli_ok` (incl. `canary premerge`). |
| `orchestration/loop/tests/console_final_console.rs:284` | `scope_and_supervisor_flag_branches` | 假阳性 | `cli_ok` ×2. |
| `orchestration/loop/tests/console_final_console.rs:302` | `delivery_record_verified_and_empty_projection` | 假阳性 | 4× `cli_ok`. |
| `orchestration/loop/tests/console_final_console.rs:408` | `store_verify_repair_and_bridge` | 假阳性 | 3× `cli_ok`. |
| `orchestration/loop/tests/console_final_console2.rs:34` | `authority_without_optional_flags` | 假阳性 | `cli_ok` with neither optional flag. |
| `orchestration/loop/tests/console_final_console2.rs:42` | `scheduler_liveness_threshold_parse_edges` | 假阳性 | `cli_ok` on the non-numeric/zero threshold arms. |
| `orchestration/loop/tests/console_final_console2.rs:90` | `include_experimental_pass_through_covers_unmatched_key_edges` | 假阳性 | 4× `cli_ok`. |
| `orchestration/loop/tests/console_sweep2.rs:42` | `agent_list_with_lease_events` | 假阳性 | 8× `cli_ok`. |
| `orchestration/loop/tests/console_sweep2.rs:166` | `privacy_stale_cache_arm` | 假阳性 | `cli_ok` ×2. |
| `orchestration/loop/tests/console_sweep2.rs:176` | `scope_with_open_gate` | 假阳性 | `cli_ok` ×3. |
| `orchestration/loop/src/console.rs:9452` | `print_obligation_with_and_without_todo` | 合理豁免 | `print_obligation` (`:6123`) returns `()` and writes to stdout. Stable Rust cannot capture a `println!` from inside the process (libtest's capture is not exposed), so the only available claim is "both arms render without panicking". Covers the `todo_id=Some/None` if/else. |
| `orchestration/loop/src/console.rs:9547` | `print_goal_status_full` | 合理豁免 | same shape for `print_goal_status` (`:3270`): covers the satisfied/open-gap, monitor-metadata, projection-gap and history print arms; no return value to assert. **rev**: a `format!`-based `goal_status_lines()` would make it assertable, but that is a production refactor, out of this audit's scope. |

### agent

| file:line | test | class | judgement (evidence) |
|---|---|---|---|
| `agent/src/config/providers.rs:1429` | `upsert_restores_models_when_models_write_fails` (`#[cfg(unix)]`) | 假阳性 | body is `let _ = (!skip_if_root()).then(upsert_models_write_fails_body);`; the real assertions are in `upsert_models_write_fails_body` (`:1437`, asserts at `:1450` and `:1452`). |
| `agent/src/config/providers.rs:1457` | `upsert_restores_models_when_auth_write_fails` (`#[cfg(unix)]`) | 假阳性 | `upsert_auth_write_fails_body` (`:1462`, asserts at `:1478`, `:1480`). |
| `agent/src/config/providers.rs:1508` | `delete_restores_models_when_models_write_fails` (`#[cfg(unix)]`) | 假阳性 | `delete_models_write_fails_body` (`:1513`, assert at `:1522`). |
| `agent/src/config/providers.rs:1527` | `delete_restores_models_when_auth_write_fails` (`#[cfg(unix)]`) | 假阳性 | `delete_auth_write_fails_body` (`:1532`, asserts at `:1542`, `:1544`). |
| `agent/src/config/providers.rs:1795` | `upsert_auth_write_fails_without_models_change_skips_restore` (`#[cfg(unix)]`) | 假阳性 | `upsert_auth_write_fails_no_models_body` (`:1800`, assert at `:1817`). (The `#[cfg(windows)]` twin at `:1334` asserts inline and was never reported.) |
| `agent/src/models/mod.rs:1771` | `model_accepts_images_returns_bool` | **真弱 → 已修** | the name promises a bool result and the body does `let _ = result;` (`:1775`) — nothing is checked, and the comment says so out loud. **Add**: `assert!(super::model_accepts_images("gpt-4o"))` for a known vision model **and** `assert!(!super::model_accepts_images("no-such-model-xyz"))` (the `unwrap_or(false)` arm at `:294`). |
| `agent/src/models/mod.rs:1880` | `get_default_model_returns_something` | **真弱 → 已修** | `let model = ...; let _ = model;` (`:1845-1847`). The stated claim ("returns something") is never checked. **Add**: either make the test hermetic (stub `AuthStore`) and `assert!(model.is_some())`, or state the environment-independent invariant `assert!(model.is_none() \|\| Registry::new().resolve(&model.unwrap()).is_some())`. |
| `agent/src/models/mod.rs:2028` | `registry_resolve_scope_with_star` | **真弱 → 已修** | `let _ = scope;` (`:1939`). **Add**: `assert!(!scope.is_empty())` for a registry+auth fixture that has one configured provider, and `assert!(reg.resolve_scope(&["zzz-no-such".to_string()], &auth).is_empty())` for the no-match arm. |
| `agent/src/rpc/prompt_helpers.rs:797` | `approve_tool_path_write_and_edit` | **真弱 → 已修** | was: comment = "Should not panic"; the only effect is `crate::tools::approve_outside_path` (`agent/src/tools/mod.rs:117`) pushing into a task-local with **no public accessor**. **Fixed** (accessor decision taken): `is_approved_outside_path` (`agent/src/tools/mod.rs:1686`) is now `pub(crate)` — visibility only, signature and behaviour untouched — and the test is a `#[tokio::test]` inside `with_tool_scope`: the fresh scope is empty, `write` with `{"path":"test.txt"}` approves the workspace's `test.txt`, and `edit` with a **different** file (`edited.txt`) is asserted separately, so a `write`-only guard fails. |
| `agent/src/rpc/prompt_helpers.rs:836` | `approve_tool_path_other_tools_noop` | **真弱 → 已修** | was: the early return at `:416` for `read`/`shell` is the point, but nothing observed it. **Fixed**: inside `with_tool_scope`, `read` (with a `path` argument) and `shell` leave the approved list empty, and a control `write` in the same scope does record the same path — so the emptiness is the tool-name guard, not a dead list. |
| `agent/src/rpc/prompt_helpers.rs:871` | `approve_tool_path_no_path_field` | **真弱 → 已修** | was: nothing asserted that the `argument_path`-`None` early return (`:420`) left the list empty. **Fixed**: inside `with_tool_scope`, `write` with `{}` and with a present-but-non-string `{"path":42}` leave the list empty, then the control `write` with a string path records it. |
| `agent/src/sandbox/mod.rs:1458` | `legacy_bash_probe_runs_real_bash_version_check` | **真弱 → 已修** | `let _ = legacy_bash_probe("bash");` (`:1462`) discards the bool. The host-dependent half is excusable, but the **deterministic** name guard (`:750`) is free: **add** `assert!(!legacy_bash_probe("sh"))` and `assert!(!legacy_bash_probe("/bin/zsh"))`. |
| `agent/src/sandbox/mod.rs:2061` | `hydrate_skips_when_shell_spawn_fails` (`#[cfg(not(windows))]`) | **真弱 → 已修** | `hydrate_from_login_shell()` (`:789`) applies PATH via `std::env::set_var("PATH", …)` (`:848`); on a failed spawn it returns at `:813`. **Add**: snapshot `std::env::var_os("PATH")` before and `assert_eq!` after — the test currently passes even if the spawn-failure path wrongly rewrote PATH. |
| `agent/src/sandbox/mod.rs:2085` | `hydrate_ignores_dump_without_marker` (`#[cfg(not(windows))]`) | **真弱 → 已修** | `plan_env_merge` returns `(None, vec![])` for a marker-less dump (`mod.rs` test at `:1996` proves it), so PATH must be untouched; nothing asserts it. **Add** the same PATH before/after `assert_eq!`. |
| `agent/src/sandbox/mod.rs:2109` | `hydrate_times_out_and_kills_hung_shell` (`#[cfg(not(windows))]`) | **真弱 → 已修** | the shell is `exec sleep 30`, so `rx.recv_timeout(5s)` expires and the timeout arm (`:836-838`) logs, `child.kill()`s and returns — **before** `plan_env_merge`, so PATH must be untouched, and nothing checks that. **Add** the PATH before/after `assert_eq!`. **rev**: the row's *second* claim (the shell was actually killed) is not observable from the test at all — only the timeout is, and it is self-detecting (a regression that hangs fails the test via the harness). If the kill needs its own coverage it must be a production-side seam, not an assertion here. |
| `agent/src/skills/manager.rs:1204` | `an_archive_that_declares_more_than_the_extraction_limit_is_refused` | 假阳性 | delegates to `install_rejects` (`:1703`), which asserts the message **and** `manager.list_installed()` emptiness (`:1708`, `:1712`). |
| `agent/src/tools/mod.rs:2953` | `approve_outside_path_adds_to_approved_list` | **真弱 → 已修** | the name promises the list is updated, but the call runs **outside** any `TOOL_SCOPE`, so `try_with` (`:121`) silently no-ops; the comment admits it. **Add**: `with_tool_scope(ScopeOptions::default(), async { approve_outside_path("/tmp/test"); assert!(is_approved_outside_path(Path::new("/tmp/test"))); })` (`is_approved_outside_path` is in-module, `:1686`). |
| `agent/src/sandbox/windows/capability.rs:662` | `save_atomic_cleans_up_temporary_on_write_failure` | 假阳性 | `save_atomic_cleanup_body` (`:667`) asserts the error **and** that no `.tmp` survived (`:673`, `:675`). |

### channels

| file:line | test | class | judgement (evidence) |
|---|---|---|---|
| `channels/src/tls.rs:76` | `platform_tls_config_builds` | 合理豁免 | `platform_tls_config()` (`:34`) returns `rustls::ClientConfig`, which exposes no public invariant to assert; the real claim is "the platform-verifier builder does not panic once a crypto provider is installed" (`ensure_provider()`, `:46`). Neighbouring tests do assert (`http_client_builds` `:56`, `ws_connector_is_rustls` `:72`). |
| `channels/tests/channel_bin.rs:113` | `no_channels_enabled_sigint_exits_cleanly` | 假阳性 | `sigint_shutdown_case` (`:94`) spawns the real binary and asserts `status.success()` after SIGINT (`:108`). |
| `channels/tests/channel_bin.rs:119` | `enabled_channel_with_dead_agent_sigint_exits_cleanly` | 假阳性 | same helper; asserts clean shutdown with a dead agent. |
| `channels/tests/channel_bin.rs:131` | `dingtalk_enabled_with_dead_agent_sigint_exits_cleanly` | 假阳性 | same helper. |
| `channels/tests/channel_bin.rs:142` | `both_channels_enabled_sigint_exits_cleanly` | 假阳性 | same helper. |
| `channels/tests/channel_bin.rs:153` | `disabled_channels_sigint_exits_cleanly` | 假阳性 | same helper. |
| `channels/tests/channel_bin.rs:215` | `a_disabled_terminal_channel_does_not_start` | 假阳性 | `sigint_case_with_home` (`:223`) asserts `status.success()` (`:235`). |
| `channels/src/feishu/feishu_ws.rs:606` | `ignore_event_is_callable` | 合理豁免 | the subject is literally `fn ignore_event(_: FeishuEvent) {}` (`:603`) — a deliberate no-op with no return value and no state; "is callable" is the whole contract. |
| `channels/src/providers/cli.rs:381` | `the_pump_stops_when_the_consumer_goes_away` | **真弱 → 已修** | `pump_lines` (`:86`) returns `()`; with `Cursor::new(b"one\ntwo\n")` it reaches EOF (`:91`) regardless of whether the closed-consumer arm (`:93`) ran, so the named behaviour is not distinguished — and nothing is asserted. **Add**: feed a reader that never ends (e.g. `BufReader::new(std::io::repeat(b'\n'))`) and assert `pump_lines` returned promptly once `rx` was dropped, or change `pump_lines` to return an enum and assert the `ClosedConsumer` variant. |

### cli

| file:line | test | class | judgement (evidence) |
|---|---|---|---|
| `cli/src/test_env.rs:16` | `lock_env` | 假阳性 | *(this name is a scanner artifact, not a test)* `lock_env` (`:16`) is a 2-line helper. The scanner matched the literal `` `#[tokio::test]` `` inside the module doc comment at `cli/src/test_env.rs:13` and attributed the next `fn` to it. The file's real test is `wait_for_exhaustion_returns_false` (`:120`), which asserts at `:121`. |
| `cli/src/browser/chromium/cdp_event_router.rs:268` | `dispatch_without_any_handlers_is_a_noop` | 合理豁免 | `CdpEventRouter::dispatch` (`:66`) returns `()`; with no handlers registered there is nothing observable to assert. The positive counterpart (`dispatches_to_matching_session_and_method` `:141`, `dispatch_with_none_session_does_not_double_fire` `:276`) asserts handler counts. |

### tui

| file:line | test | class | judgement (evidence) |
|---|---|---|---|
| `tui/src/app.rs:11343` *(drifts — see note)* | `scrollback_terminal_exit_callback_setter` | 合理豁免 | sets the callback on the test double, whose `set_exit_signal_callback` is `fn …(_cb: …) {}` (`tui/src/app.rs:9929`/`:10166`/`:11054`) — by construction no observable effect. The test pins that the double implements the whole `TerminalIo` surface (the sibling `fake_terminal_exit_callback_setter`, currently `:15317`, does the same for the other double). |
| `tui/src/crash.rs:291` | `raw_write_stderr_writes_bytes` | 合理豁免 | writes raw bytes to the **real fd 2** (`:134`/`:139`); the source comment explains that redirecting fd 2 would collide with the tests that capture it, which is exactly why no output assertion is made. The bad-fd arm (`:298`) proves the loop breaks instead of panicking. |
| `tui/src/terminal.rs:842` | `noop_callbacks_invoke` | 合理豁免 | the subjects are `noop_input_cb`/`noop_resize_cb` (`:833`/`:837`), which construct closures whose entire body is empty — "callable without panicking" is the only true statement available. |
| `tui/src/terminal_posix.rs:571` | `signal_handlers_install_and_restore` | 合理豁免 | installs/restores real POSIX signal handlers (`:127`/`:145`) under a test lock; there is no return value and no state to read back, so "no fault/deadlock" is the observable. |
| `tui/src/terminal_posix.rs:832` | `restore_raw_handles_missing_pipe` | 合理豁免 | drives `restore_raw` from a hand-built `Backend` whose pipe snapshot is `None` — the source comment says the state is not reachable through the public flow; the call returns `()` and the second call exercises the disabled early-return. |
| `tui/src/terminal_posix.rs:848` | `panic_restore_raw_skips_when_lock_held` | 合理豁免 | the claim is precisely "the poisoned/skip branch does not deadlock" — `panic_restore_raw()` must return while `PANIC_TERMIOS` is held. Any assertion would have to *not* be in the way of that lock, so completion is the assertion. |
| `tui/src/terminal_posix.rs:856` | `signal_handler_ignores_unset_pipe` | 合理豁免 | calls the `extern "C"` signal handler with `fd < 0`; the contract is "no write, no crash" — a signal handler may not assert or allocate. |
| `tui/src/components/chat_area.rs:2598` | `tiny_terminal_width_does_not_underflow` | **真弱 → 已修** | was `let _ = chat.render(width);` (`:2605`) — the rows were discarded. **Fixed** (table-driven over `width ∈ {0,1,2,3,4,40,1000}`, all assertions on the **returned** rows): (a) the `render(width)` view is non-empty and is a *window* of `render_all(width)` (an underflowed `viewport_top` slices out of range); (b) every row's visible width is `<= width.max(3)` — the underflow guard, because an underflowing `width - 2` reaches `apply_background_to_line`, whose `" ".repeat(padding)` would pad a row to ~`usize::MAX` cells; (c) every non-empty row keeps its 1-column gutter (no shift into the terminal edge); (d) squashing the rows back together recovers all three message texts, one of them CJK — clamping wraps the text, it does not drop it; (e) widths 0/1/2 render **byte-identically**, so the content-width clamp (`width.saturating_sub(2).max(1)`) is visible in the output rather than merely implied. **The `rev` was right and the suggestion was wrong**: `<= width.max(1)` is false — the minimum layout is gutter + one content column, and a wide CJK glyph fills both, so a 0-column terminal legitimately emits a 3-cell row (measured: `old[27] = " 答"` at `w=3`; the old `max(1)` bound would have failed on correct code). **Mutation-checked** (temporary, reverted): padding the user row to `width + 100` fails (b) with `row is 100 cells wide, expected <= 3`; rendering the user content as `""` fails (d) with `"helloworld" missing from the rendered rows`. |
| `tui/src/components/scoped_models_selector.rs:395` | `noop_callbacks_are_callable` | 合理豁免 | subjects are `noop_save` (`:386`, body `Box::new(\|_\| {})`) and `noop_cancel` (`:390`, `Box::new(\|\| {})`) — empty closures; nothing to assert. |
| `tui/src/rpc/grpc_client.rs:3657` | `wait_connected_helper_loops_until_true` | 假阳性 | `wait_connected` (`:1644`) delegates to `wait_conn` (`:1649`), which does `timeout(10s, …).await.expect(what).expect("conn channel closed")` (`:1650-1653`) — a timeout or a closed channel **panics**, so both connected states are asserted. (The comment-style delegation, third false-positive shape.) |

### packages

| file:line | test | class | judgement (evidence) |
|---|---|---|---|
| `packages/rpc/tests/wire_roundtrip.rs:245` | `server_constructor_variants_and_tuning` | 合理豁免 | exercises `FutureAgentServer::from_arc` / `with_interceptor` and the tonic tuning builder chain; each call returns `Self`, which has no public invariant to assert. The wire behaviour itself is asserted by the neighbouring round-trip tests (`:80`, `:126`, `:213`, `:257`). |

---

## Anti-pattern sweep — was any *production* line changed for the number?

*(Independent verification pass, 2026-09-26. The supervisor asked this after
`docs/testing/module-tauri-terminal.md` §11 **proposed** deleting a redundant
DSR guard in `terminal/server.rs` "to remove these five lines": "because a line
was uncovered" is exactly the anti-pattern the goal forbids, so the question was
whether that class of change — a production guard deleted to raise coverage —
was actually made anywhere.)*

**Answer: no. Not one deleted production line in the diff removes a guard, an
error arm or a `return` to raise the number.** The DSR deletion was proposed and
**not** carried out. Details, and the one place the proposal's reasoning is
wrong, below.

### Method (reproducible)

The goal's work is *uncommitted* on branch `test/cov100` (`HEAD` = `c173e64e`,
`origin/main`); `git diff HEAD` is the whole change: 280 files, +37 652 / −850.
Every deleted line is classified production-vs-test by locating the file's first
`#[cfg(... test ...)]` attribute in the `HEAD` revision and comparing the
deletion's original line number against it, with obvious test-file paths
excluded by name:

```python
import subprocess, re, collections
TSKIP = re.compile(r"(^|/)(tests?|test_support)\.rs$|_tests\.rs$|/tests/|\.test\.|__tests__")
BOUND = re.compile(r"\s*#\[cfg\(\s*(all\()?\s*[^)]*\btest\b")
d = subprocess.run(["git","diff","HEAD","-U0","--","*.rs"],capture_output=True,text=True).stdout
# walk hunks: @@ -old,count +new,count @@ gives the original line number of the first deletion
# then: test if rel-path matches TSKIP, or old line >= first BOUND line in `git show HEAD:<path>`
```

Result: **850 deleted lines = 779 in test code or comments + 71 production
lines**, the 71 spread over 11 files.

### The 71 production deletions, adjudicated

| file | deleted | what it was | verdict |
|---|---|---|---|
| `orchestration/loop/src/console.rs` | 39 | the detached-run re-exec body: the `child_args` build and the `try_wait` liveness guard (`Ok(Some)` → `bail!`; `Ok(None)`/`Err` → announce and continue) | **refactor, not removal.** The code reappears verbatim as `detached_child_args()` (`console.rs:8794`) and `enforce_detach_liveness()` (`:8811`), both called from the original site; the three outcomes are preserved (`Ok(None) | Err(_) => { println!(…); Ok(()) }` — the old code reached the same `println!` by falling out of two empty arms). New tests: `detached_child_args_prepend_the_group_only_for_the_unified_binary`, `detach_liveness_guard_decides_from_the_probe_outcome`. |
| `desktop/src-tauri/src/windows_power.rs` | 14 | the three `else if` conditions of the wndproc power handler (`WM_POWERBROADCAST`+`PBT_APMSUSPEND`; the three `PBT_APMRESUME*` codes; `WM_QUERYENDSESSION`) | **refactor, every constant preserved.** Extracted to the pure `power_action(message, wparam) -> PowerAction` (`windows_power.rs:28`); the diff adds the `Ignore` arm and a `WM_QUERYENDSESSION \| 0x8000` counter-case that did not exist before. `install_disconnect_notifier` gained `<R: tauri::Runtime>` (testability, same as `scheduler::start`). |
| `orchestration/loop/src/compat.rs` | 3 | `Err(error) if error.kind() == AlreadyExists => {}` and two `remove_lock_file(..)?` | **widened, not removed.** The guard became `create_is_contention(&error)` = `AlreadyExists` ∪ (Windows) `ERROR_ACCESS_DENIED`; each `?` became an explicit `if let Err(..) { if delete_pending_retry(..) { continue } return Err(..) }` — the error is still returned. New tests pin both the failure and the retry, including a Windows sharing-violation counter-case. |
| `channels/src/session_store.rs` | 1 | `pending.persist(&self.path)?` | **wrapped, not removed.** `persist_with_retry(pending, &self.path)?` returns the error once the budget is spent or the kind is not transient; `a_replace_that_cannot_publish_is_reported_not_swallowed` pins the failure path. |
| `agent/src/skill_reco/mod.rs` | 2 | `let query_bytes = query.trim().len();` and `match HTTP_CLIENT.post(…)` | **moved into `attempt_call(client, endpoint, …)`** so a local socket can drive every transport outcome. `query_bytes` is still computed and used; the client is now a parameter. |
| `cli/src/browser/windows_process.rs` | 2 | the `Start-Process` format line | **replaced by an equivalent format** that omits `-ArgumentList` for an empty argv — a real bug fix (PowerShell 5.1 rejects `-ArgumentList ''`). New test `empty_argv_omits_the_argument_list_switch`. |
| `channels/src/dingtalk/bridge.rs` | 3 | the macro arguments of one `info!` | **hoisted** into `let sender_name = …` so the expression is evaluated even with no subscriber installed (matching `feishu::bridge`). Same values. |
| `desktop/src-tauri/src/scheduler/mod.rs` | 1 | `pub fn start(app: tauri::AppHandle)` | **signature generalised** to `<R: tauri::Runtime>` so a `tauri::test::mock_app()` handle can drive it. No arm removed. |
| `agent/src/sandbox/linux/{runner,request}.rs` | 2 + 2 | two doc comment lines before `#[cfg(all(test, unix))]` | **comments moved**; the attribute itself survives (see the `+` twin) and production code is untouched. |
| `agent/src/sandbox/linux/glob_scan.rs` | 2 | same | same |

Nothing else in the 850 deleted lines is production code.

### The DSR guard: proposed, **not** executed — and the proposal is unsound

* **Not executed.** `answered_dsr` guards are still present three times:
  `server.rs:1335` (the end-to-end readiness loop) and `:1952` + `:1982` (the
  pump test's two loops). `git diff HEAD` shows all three arriving as `+` lines
  on this branch; nothing deletes either guard body. §11's five uncovered lines
  remain **waived**, not removed.
* **The proposal's premise is wrong.** §11 argues the second guard's body cannot
  execute because both guards share one `answered_dsr` flag and the first guard
  "fires (the shell's query arrives as the session starts)". That is an
  observation about *this host's* interleaving, not a construction:
  the first loop's exit condition is `pong.is_none()`, and the **Pong comes from
  the pump's transport half in reply to the client's `Ping`**, with no PTY
  involvement, while the `ESC [ 6 n` query comes from the *shell* after it
  starts. Nothing orders those two; if the Pong is observed first the first loop
  exits with the flag still `false`, and the **second** guard is then not merely
  reachable but *required* — without it the shell never gets its reply and the
  marker loop is the thing that hangs. So the correct category for those lines
  is at most `unreachable-in-this-environment` (host-dependent ordering), never
  `unreachable-by-construction`; and deleting the second guard would remove the
  only answer for a legal interleaving, i.e. it would trade a ledger line for a
  latent hang on a slower host. **The proposal should be rejected**, and the
  waiver category corrected; a timing-independent fix (one shared
  "answer the cursor query" helper consulted by both loops, or answering
  idempotently without the flag) would exercise the path instead of waiving it.
  *(The five line numbers in §11 no longer resolve — the file has grown since —
  so this pass matched the guards by content, not by number.)*

### `#[cfg(test)]` in production functions (seams, not wrapped branches)

The goal adds test-only hooks to production files. **None of them wraps an
existing production branch in `#[cfg(test)]`**; each is an additive early hook
whose production path is unchanged. They are listed here so a reviewer can judge
them rather than discover them:

| location | hook |
|---|---|
| `orchestration/loop/src/console.rs:8842` | `try_wait_detached`: returns a `detach_probe`-injected `io::Result` when armed, else `child.try_wait()`. The one seam that interposes on a *production* value; the test also asserts the real OS read, so the seam is not shadowing it. |
| `orchestration/loop/src/store.rs:803-804` | `crate::store::write_fault::maybe_fail(&event)?` — the fault injector the ledger-write tests arm. |
| `agent/src/rpc/mod.rs:340-352` | `ACTIVATE_PERSISTED_SESSION_FAIL_HOOK` — injects a hydrate failure after the caller committed the session. |
| `agent/src/rpc/protocol.rs:584` | `EventWriteCommand::Hold(gate)` — a `#[cfg(test)]` enum variant plus its arm, used to order the journal writer against the command loop. |
| `agent/src/rpc/protocol.rs:598` | `#[cfg(test)] commits: &AtomicU64` parameter on `flush_event_batch`. |
| `agent/src/rpc/session_prompt.rs:775`, `:888` | capture handles on the run's callbacks (`RunCallbacksForTest`). |
| `desktop/src-tauri/src/terminal/{manager,session}.rs` | `session_for_test`, `child_is_reaped`, `wait_for_child_exit` — `pub(crate)` accessors, no behaviour added. |
| `orchestration/loop/src/agent_client.rs:142` | `unreachable_for_test()` — a lazy client to a dead port. |

Each row's anchor is the hook's `#[cfg(test)]` **attribute** line (or the range
including its body), not a `fn` line: this table lists code fragments, so the
row-anchor script in the reviewer checklist does not apply to it. All six
anchors were re-read against the file after the edits and resolve.

A seam that lets a test reach an error arm is strictly better than deleting the
arm or waiving it, and every one above is documented as test-only in place. The
residual risk is that the *armed* arm is what a test exercises while the
*unarmed* path is not — the `console.rs` seam guards against that by asserting
the real read too.

### Deleted / reshaped assertions (weakening check)

Every deleted line that mentions `assert` was read in context. Twenty were in
test code; **none is a dropped claim**:

| file | deleted assert | what replaced it |
|---|---|---|
| `agent/src/rpc/session.rs` | `assert!(result.is_err())` ×2 (compaction) | stronger: the tests now prove the *degraded* compaction succeeds (`evidence_only`, checkpoint committed, not charged) and, separately, that a failed receipt is durable and refuses a retry. The old pair is called out in-source as vacuous — it passed on "session does not exist" without ever reaching the provider. |
| `agent/src/rpc/session.rs` | `set_thinking_level_updates_field` / `_off` | replaced by one table-driven test over all six levels + an unknown one, asserting the **budget that reaches the provider**, not just the echoed string. |
| `agent/src/rpc/session.rs` | `default_workspace_contains_future_agent`, `set_cwd_updates_field`, `get_permission_level_default` | re-added unchanged (the file was reorganised). |
| `agent/src/rpc/session.rs` | `set_permission_level("invalid")` with "should not crash" | replaced by `assert_eq!(get_permission_level(), "invalid")` — an actual claim. |
| `orchestration/loop/src/work_items/replan_obligation.rs` | `assert!(…iter().all(\|o\| o.kind != X))` ×3 | replaced by `assert!(…is_empty())`; the in-source comment names the old form as **vacuous** (the closure never ran on an empty list, so it could not fail). Strengthening. |
| `agent/src/sandbox/linux/plan.rs` | `assert!(matches!(expand_glob(&p), Err(..) if …))` | same match, bound to a name and given a failure message. |
| `cli/src/utils/files.rs` | `assert!(found.is_some())` + `contains(expected)` in one `cfg!`-branched test | split into `#[cfg(windows)]` / `#[cfg(unix)]` twins with **identical** assertions — the runtime `cfg!` branch would have left the other platform's lines permanently uncovered. |
| `tui/src/app.rs` | `assert!(last_system(&app).contains(…))` ×2 | `pump_until_msg(…)` then `.any(\|m\| m.contains(…))` — waits on the message instead of assuming it is last (a real race between two refused loopback connects). |
| `tui/src/app.rs` | `assert_eq!(seen, vec![<exact path string>])` ×2 | `assert_eq!(seen.len(), 1)` **+** `assert_eq!(location(&seen[0]), location(&expected))`, where `location()` (`app.rs:11783`) normalises `\`→`/`, strips the `\\?\` verbatim prefix and lowercases on Windows. Content equality is kept; only the *spelling* is tolerated (the app echoes its own `Path::join` separators, git reports its own). Not a dropped claim. |
| `desktop/src-tauri/src/commands/remote.rs` | `assert!(result.is_err())` behind a read-only-dir setup | replaced by a stronger triple: the error is the **local** one (`"Not signed in"`), the legacy credential is gone, and it was present before the call — which is only consistent with `clear_creds()` actually running. |
| `desktop/src-tauri/src/remote/commands.rs` | `assert!(now < deadline, "run never settled")` | moved into the shared `wait_until(…)` helper with the same deadline; the `assert_eq!(status, "completed")` after it is new. |
| `desktop/src-tauri/src/terminal/{pty,session,manager}.rs` | 9 asserts inside `#[cfg(unix)]` tests that drove `/bin/sh` | the tests were rewritten onto `terminal::test_support` platform-neutral fixtures, **keeping** the `#[cfg(unix)]` attribute off so they now run on Windows too (this is the platform red line being fixed, not skipped). |

### Defects found by this pass (reported, not fixed — outside the audit's write scope)

1. **Stale comment on a real test.** `desktop/src-tauri/src/commands/remote.rs:191-197`
   says the uncategorized local fault "is produced by putting a *directory* where
   the credential file belongs". The test body does not do that — it writes a
   normal legacy credential and relies on the **"Not signed in"** fault during
   re-pairing (the comment's own second paragraph). The first paragraph is a
   leftover from the deleted `#[cfg(unix)]` read-only-dir approach and should be
   dropped; it does not affect the assertions, which are correct and strong.
2. **Waiver category.** The `terminal/server.rs` DSR row (§11, and the
   corresponding line in `docs/testing/waiver-ledger.md`) is filed
   `unreachable-by-construction`; per the counterexample above it should be
   `unreachable-in-this-environment`, or the test should be made
   timing-independent. This is a ledger-category fix, not a code change.

---

## Reviewer checklist

* For each **真弱** row: the suggested assertion names the exact function/arm it
  would pin; the module owner adds it, the audit row is then closed. All 13
  `真弱 → 已修` rows are closed (re-checked with `python .future/cov100/verify.py
  weak`); none of the 13 names appears in its worklist.
* For each **假阳性** row: confirm the named helper really asserts (the file:line
  is given for every one).
* For each **合理豁免** row: confirm the subject genuinely has no observable
  surface (no-op body, `()` return with no state, or an unsafe handler).
* Two rows are flagged **rev** (`chat_area.rs:2598`, `console.rs:9490`): the
  classification is defensible, the *fix suggestion* is the debatable part. The
  `chat_area.rs:2598` one was re-derived when the fix landed and the suggestion
  was **wrong** (bound `width.max(1)`; the real minimum is 3 cells) — treat the
  other `rev` row's suggestion the same way, as a hypothesis to check against
  the measured output.
* **Line numbers drift while this goal runs.** Several workers edit this
  checkout concurrently, and `tui/src/app.rs` alone gained 1318 lines *after*
  this audit was written — moving `scrollback_terminal_exit_callback_setter`
  from 11278 to 11343 and `fake_terminal_exit_callback_setter` from 15251 to
  15317. The **test name is the stable key**: locate the test by `fn <name>`
  and treat the line number as a hint. A re-run of the check below re-derives
  every anchor; **as of the independent verification pass (2026-09-26) exactly
eight rows had drifted** and were refreshed in this pass — `console.rs`
  `print_obligation_with_and_without_todo` 9395→9452 and `print_goal_status_full`
  9490→9547, `agent/src/models/mod.rs` `get_default_model_returns_something`
  1844→1880 and `registry_resolve_scope_with_star` 1934→2028,
  `agent/src/sandbox/mod.rs` `hydrate_skips_when_shell_spawn_fails` 2049→2061,
  `hydrate_ignores_dump_without_marker` 2064→2085 and
  `hydrate_times_out_and_kills_hung_shell` 2080→2109, and
  `channels/src/providers/cli.rs` `the_pump_stops_when_the_consumer_goes_away`
  342→381. The other 61 rows anchored exactly. Re-run:

  ```python
  import re
  from pathlib import Path
  text = Path('docs/testing/weak-test-audit.md').read_text(encoding='utf-8')
  for f, ln, name in re.findall(r'^\| `([^`]+\.rs):(\d+)` \| `(\w+)`', text, re.M):
      lines = Path(f).read_text(encoding='utf-8', errors='replace').splitlines()
      hits = [i + 1 for i, l in enumerate(lines) if re.search(r'\bfn\s+' + name + r'\b', l)]
      if int(ln) not in hits:
          print('DRIFTED', f, ln, '->', hits, name)
  ```

## Honest limitations

* This audit is source-level: it does not decide whether a test's assertions are
  *strong enough*, only whether an assertion exists at all.
* Several offenders are behind `#[cfg(unix)]` / `#[cfg(not(windows))]` and never
  compile on the Windows host this was written on (the hydrate and POSIX-signal
  rows). Their classification is by reading, not by running.
* No test or source file was changed for this audit. Adding the suggested
  assertions is a follow-up for each module owner.
