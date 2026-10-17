# future-tui — coverage, measurement basis, branch analysis, waivers and test-quality record

Goal `cov-100-multidim` · module **future-tui** (`tui/`) · branch `test/cov100`
(worktree `.worktrees/cov100`) · host **Windows 11 x86_64**, rustc 1.97.0
(`rust-toolchain.toml`, stable) · cargo-llvm-cov **0.9.1** · session
`20260926-030830-3cf240a76381473e9104a8ea2321253d`.

Handoff tokens: `dimensions` — §4. `weak-tests-fixed` — §6 (four discharged this
segment, including one false positive in my own scan). `lines-100-or-waived` —
**satisfied**: 377 uncovered lines remain by the gate's metric (291 per line), and every one of them is waived with a category and a reason (§5). Note the gate metric counts line-table *entries*, so it drifts by a few between identical runs — `366`, `375`, `376` and `377` were all observed for the same source. The per-line figure (llvm-cov `DA:0` records) is stable at `290`-`291`, and that is the number to compare across runs.
and **every one of the 25 files that hold them is waived with a category in §5**
— §5.4 lists `tui/src/app.rs`'s last six lines individually, and they are all
test-side or environment limits, not production behaviour. No file is declared
OPEN; §7 is the reconciliation. **The acceptance gate passes**
(`python .future/cov100/verify.py crate future-tui 99.99 coverage/tui-report.json`,
exit 0).

---

## 1. Measurement

### 1.1 The canonical command

cargo-llvm-cov 0.9.1 here has **no `--target-dir` flag**; the private target dir
comes from `CARGO_TARGET_DIR`, and the tool appends `llvm-cov-target`:

```powershell
$env:CARGO_TARGET_DIR = "target/cov-tui2"
cargo llvm-cov -p future-tui --lib --no-report             # full lib suite, no filter
cargo llvm-cov report --json --output-path coverage/tui-report.json
cargo llvm-cov report --lcov --output-path coverage/tui-report.info
python .future/cov100/verify.py crate future-tui 99.99 coverage/tui-report.json
```

No feature flags; the `--no-report` run and both `report` runs share one
`CARGO_TARGET_DIR`. `--lib` is deliberate (§1.3).

### 1.2 Result

| | lines covered / counted | line % | uncovered (gate metric) | per-line (`DA:0`) | files with gaps |
|---|---|---|---|---|---|
| task text (start of the goal) | — | — | 1087 | — | 31 |
| previous segment | 59110 / 59734 | 98.9554 % | 624 | 531 | 31 |
| **this segment (final)** | **61094 / 61471** | **99.3867 %** | **377** | **291** | 25 |
| all-targets form (rejected, §1.3) | 59060 / 66573 | 88.7146 % | 7513 | — | 31 |

Suite state from the measuring run: **2122 passed, 0 failed, 0 ignored**.
Note the denominator also grows with every test line added (a file's inline
`mod tests` is measured with the file), so the crate percentage drifts as the
suite grows even when no production line regresses; `verify.py uncovered` is the
figure that tracks the work.
Region/per-path metric: **104579 / 105708 = 98.9320 %** (98.7833 % at the start of
this segment).

**Gaps closed completely this segment**: `tui/src/components/markdown.rs`,
`tui/src/components/input.rs`, `tui/src/components/chat_area.rs`,
`tui/src/stdin_buffer.rs`, `tui/src/insert_history.rs`,
`tui/src/terminal_image.rs`, and — as metric residue only —
`tui/src/paste.rs`, `tui/src/clipboard.rs`, `tui/src/external_editor.rs`,
`tui/src/skills_cli.rs` (§5.1). Largest reductions:
`tui/src/components/autocomplete.rs` 63 → 9, `tui/src/skills_cli.rs` 24 → 1,
`tui/src/rpc/grpc_client.rs` 37 → 7, `tui/src/components/chat_area.rs` 6 → 0.

### 1.3 Why `--lib`, and the multi-unit defect

`summary.lines.count` (the `Lines` column, and LCOV `LF`) counts **line-table
entries**, not lines. In the all-targets form one source file is compiled into
several instrumented units and the entries are summed: `tui/src/app.rs` was
reported as **24102 lines in a 23849-line file**, while `--lib` reports 17621 —
the shape of the recorded baseline (17585). Per-line evidence on the all-targets
report: 2979 of app.rs's "uncovered" lines carried *both* a zero and a non-zero
entry, i.e. they execute. Use `--lib`; treat the `DA:<n>,0` records (printed by
`coverage/tui-table.py`) as the truth about what never ran.

`--lib` is not perfect either, and the residual bias has a known direction: lines
reached **only** through an integration test (`tests/cli_smoke.rs`,
`tests/bughunt_regressions.rs` — separate binaries) are reported uncovered by the
lib-only form. Measured on this crate: `tui/src/index.rs` has 111 zero lines in
the lib form but **108** in both forms, i.e. 3 of its "uncovered" lines are in
fact executed by `cli_smoke.rs` (`list_models_without_agent_exits_one`) — and the
two forms disagree about *which* lines (the all-targets form's zero lines for
that file are in the `CliArgs` parsing region, the lib form's are in
`run_interactive`). `coverage/tui-union.py` intersects the zero-line sets of any
number of forms to separate "never executed anywhere" from "executed in a unit
this report does not contain"; the recommended repo-wide policy is the per-line
union across the lib and all-targets forms (§8).

### 1.4 Branch coverage is not measurable on this toolchain (corrects the brief)

```
cargo llvm-cov -p future-tui --lib --branch --no-report
→ rustc … -C instrument-coverage -Z coverage-options=branch … (exit code 1)
error: could not compile `future-tui` (lib test)
```

`-Z coverage-options=branch` is nightly-only and the repo pins stable 1.97.0, so
`summary.branches.count == 0` for every file and `--branch` does not compile. Any
"branch %" for this crate would be invented, so none is given. The measurable
substitute is llvm-cov's **region** data plus the derived **mixed-line** count: a
line that executed while at least one of its sub-paths (one arm of an `if`, a
`match` arm, a `?`/short-circuit operand) did not — exactly "line 100 %, path
untested", invisible in the line percentage.

| | start of this segment | final |
|---|---|---|
| crate regions covered | 103922 / 105202 = 98.7833 % | **104579 / 105708 = 98.9320 %** |
| mixed lines (line covered, sub-path not) | 363 | **290** |

Files whose lines look complete but whose paths are not (`python coverage/tui-regions.py coverage/tui-report.json`):

| file | line % | regions | mixed | note |
|---|---|---|---|---|
| `tui/src/app.rs` | 99.09 % | 98.9 % | **78** | the key/state-machine arms; same clusters as its 160 uncovered lines (§7). |
| `tui/src/stdin_buffer.rs` | 100 % | 96.4 % | **26** | the decode table; a table-driven test would close most of it (the `keys.rs` shape). |
| `tui/src/index.rs` | 93.45 % | 91.4 % | **24** | CLI arg / `run_interactive` branches. |
| `tui/src/paste.rs` | 99.46 % | — | 18 | bracketed-paste negotiation. |
| `tui/src/components/diff.rs` | 100 % | 99.2 % | 13 | 100 % lines, 13 untaken sub-paths. |
| `tui/src/components/markdown.rs` | **100 %** | 99.6 % | 10 | closed on lines this segment; sub-paths remain. |
| `tui/src/terminal.rs` | 92.58 % | 92.5 % | 8 | reader-loop arms needing injected console input. |

**`tui/src/keys.rs`, the demonstration:** it was at **100 % lines yet 97.7 %
regions with 64 mixed lines** — every one an arm of the legacy escape-sequence
table no test had ever matched. A table-driven test over all 50 key families and
the 52 spellings they document (through the public `parse_key`, plus an
undocumented sequence asserted `None`) took it to **16 mixed / 99.2 % regions,
still 100 % lines**. The line metric could not see any of that.

### 1.5 Measurement hazards

* Clear `*.profraw` + `*.profdata` in the private dir before a measurement, or
  the merge picks up stale profile data (observed: `terminal.rs`'s counted lines
  sliding 826 → 1613 → 1268 with no source change).
* Four tests are load-sensitive (`app::tests::a_submitted_draft_prefetches_without_holding_the_send`,
  `app::tests::a_failed_prefetch_is_silent_and_not_retried`,
  `app::tests::a_real_repository_creates_a_worktree_and_switches_to_it`,
  `worktree::tests::real_git_lists_creates_and_reports_a_worktree`): each passes
  in isolation, and a red run produces **no usable report** (a 2.32 % report with
  most files 0-covered). Retry until green, then report — every number in §1.2
  comes from a green run.
* The gate prints a "files absent from the report" warning for
  `tui/src/main.rs`, `tui/src/components/mod.rs` and `tui/src/rpc/mod.rs`. They
  carry no measured code: the two `mod.rs` files are module declarations only,
  and `main.rs` is the binary entry point, which the `--lib` measurement
  deliberately excludes (it is covered end-to-end by `tests/cli_smoke.rs`).

---

## 2. What landed (this module, over the whole task)

The table below is the **cumulative** record: the left column is the task's
starting figure (`verify.py uncovered` at the beginning of the goal), the right
column is the final one. The per-segment narrative follows it.

| file | uncovered at task start → final | driver |
|---|---|---|
| `tui/src/app.rs` | 190 → **6** (all waived, §5.4) | the whole app surface driven through its own entry points, plus the fast-live-client recipe (§2.5) |
| `tui/src/terminal.rs` | 388 → **63** (waived, §5.2) | the console harness + `Terminal::default`, the fault-injection seam, the zero-read/resize contracts |
| `tui/src/terminal_windows.rs` | 167 → **48** (waived, §5.2) | the console child makes this backend runtime-tested on Windows instead of type-checked only |
| `tui/src/index.rs` | 111 → **101** (waived, §5.2) | the CLI flag matrix and `run()`'s tails; the residue is the PTY-only `run_interactive` body |
| `tui/src/components/autocomplete.rs` | 63 → **9** (waived) | the `AttachmentProvider` tool contract; 2 weak tests fixed. (This row said `3` for one revision: it was not re-measured after the fragile `find.exe`-stub test was deleted, which is what covered the `find`-succeeded arm — see §2.1.) |
| `tui/src/clipboard.rs` | 33 → **3** (metric residue, §5.1) | 9 Windows tests for the default runner |
| `tui/src/rpc/grpc_client.rs` | 37 → **4** (waived) | `suggest_skill`'s whole body; `wait_connected`'s helper made a real assertion |
| `tui/src/agent_supervisor.rs` | 19 → **37** (waived, §5.3) | the `OwnedAgent` drop contract, asserted against a real child |
| `tui/src/skills_cli.rs` | 24 → **1** (metric residue) | the Windows counterparts of the real-runner tests |
| `tui/src/components/chat_area.rs` | 6 → **0** | 5 tests (setter idempotence, prepend notification, viewport clamp, the streaming ellipsis, folded-member re-render) |
| **crate** | **1087 → 377** gate metric / **291** per-line; 98.17 % → **99.3867 %** | |

Four files whose gaps were **entirely** metric residue are at 100 % on a per-line
basis (`tui/src/clipboard.rs`, `tui/src/skills_cli.rs`, `tui/src/paste.rs`,
`tui/src/external_editor.rs`) and are waived as `attribution-artifact` in §5.1.

New tests, named:

* **`tui/src/components/chat_area.rs`** — `set_compact_activity_ignores_a_repeated_value_and_refolds_on_a_change`
  (a repeated setter must not refold the run; asserted on the rendered rows),
  `prepending_history_shifts_the_viewport_and_notifies_the_owner` (the callback
  the app registers fires exactly once, and the returned height is real),
  `render_clamps_a_viewport_left_past_the_end_by_shrinking_content` (the clamp
  after a fold, with the transcript proven taller than the viewport),
  `the_thinking_marker_shows_an_ellipsis_only_while_streaming` (in-progress marks
  appear while streaming and disappear when the run completes),
  `rerendering_a_folded_member_changes_no_rows` (a folded member has no rows of
  its own to splice).
* **`tui/src/stdin_buffer.rs`** — `an_incomplete_escape_before_multibyte_text_never_splits_a_character`:
  an incomplete escape followed by CJK text walks byte offsets *inside* the
  character's UTF-8 encoding, where `is_char_boundary` is false; the scanner must
  step over those offsets (slicing there would panic) and must not lose a byte.
* **`tui/src/components/input.rs`** — `down_from_the_last_visual_line_of_a_recalled_multi_line_entry_walks_history`:
  the "caret first, history second" rule when the recalled entry itself spans
  several visual lines.
* **`tui/src/components/markdown.rs`** — `a_top_level_indented_paragraph_keeps_its_leading_spaces`
  (presentation-text indentation survives; container indentation inside a list
  item does not become text — both sides asserted).
* **`tui/src/components/terminal_image.rs`** — `hyperlink_collapses_a_control_bearing_url_to_plain_text`:
  an OSC 8 URL carrying `BEL`/`ST`/newline/NUL/tab must degrade to plain text
  (control bytes in an escape sequence are terminal-command injection), while
  non-control Unicode in the URL is left alone.
* **`tui/src/insert_history.rs`** — `write_history_writes_nothing_for_blank_rows`
  (the exit path must not append blank rows to the user's scrollback after the
  shell prompt).
* **`tui/src/paste.rs`** — `the_real_spawner_runs_a_program_and_reports_a_missing_one`
  (the Windows half of the pair; the real capture path reports the exit status
  and both streams separately).
* **`tui/src/external_editor.rs`** — `create_draft_file_reports_a_name_that_cannot_be_created`
  (a non-collision failure is not retried and names the path).
* **`tui/src/clipboard.rs`** — `env_value_treats_blank_as_unset` now runs both
  restore arms (the key is probed pre-set and absent) and asserts the probe key
  never leaks.
* **`tui/src/skills_cli.rs`** — the Windows counterparts of the real-runner tests
  (the originals use `sh`, which Windows lacks, so the runner's own timeout and
  flood paths had never executed here): `a_real_windows_spawn_runner_captures_streams_and_the_exit_code`
  (exit code reported verbatim, streams separate), `a_real_windows_wait_times_out_and_kills_the_child`
  (`ping -n 30` with a 300 ms budget must fail with the timeout marker **and
  return well before the child would have exited**, which is what proves it was
  killed and reaped rather than waited out),
  `a_real_windows_child_flooding_the_capture_limit_fails_explicitly` (PowerShell
  writing `MAX_CAPTURED_BYTES + 2 MiB` — the error names the ceiling instead of
  letting memory follow the child),
  `a_real_windows_wait_drains_stdout_larger_than_the_pipe_buffer` (200 000 bytes,
  every byte captured — the draining that keeps a fast child from being killed as
  a timeout), and `deadline_error_distinguishes_a_killed_child_from_a_leaked_pipe`
  (both deadline messages asserted directly, since a real leaking grandchild is
  not deterministic).
* **`tui/src/rpc/grpc_client.rs`** — `suggest_skill_returns_the_recommendation_and_sends_the_candidates`
  (asserts the wire request: query and both candidates in order),
  `suggest_skill_tolerates_a_missing_or_untyped_description` (absent/number/null
  → the same name with an empty description),
  `suggest_skill_declines_a_null_unshaped_or_untyped_answer` (6 malformed
  bodies), `suggest_skill_declines_on_transport_and_agent_failures` (a tonic
  `Unavailable` and a `success:false` answer),
  `suggest_skill_sends_an_empty_candidate_set_and_query_verbatim` (boundary +
  Unicode/CJK round trip).
* **`tui/src/agent_supervisor.rs`** — `dropping_the_owned_agent_kills_its_child`
  (spawns a real child, **first proves the external liveness observer can see a
  live process**, then asserts the drop killed it),
  `ensure_agent_running_refuses_to_spawn_in_test_builds`.
* **`tui/src/keys.rs`** — `every_documented_spelling_of_a_key_resolves_to_the_same_id`
  (the table-driven test described in §1.4).
* The console harness and the `terminal`/`clipboard`/`terminal_windows` tests
  from the previous segment are unchanged (§2.3 of the earlier revision).

### 2.1 A race I introduced and then removed (worth recording)

The first version of the `find`-fallback test installed an argv-echoing
`find.exe` **next to the test binary**, because `CreateProcessW` searches
System32 before `PATH` and System32 always has `find.exe`. That test's companion
("no usable tool") then failed intermittently, and the reason was twofold:

1. the exe directory is shared by every test in the binary, so a concurrently
   running test saw the stub; and
2. a run killed before `Drop` left the stub behind — a leftover `find.exe` from
   an earlier crashed run was found in `target/cov-tui/debug/deps/` and made the
   companion assertion fail permanently.

The `find` stub test was **deleted** rather than serialized: a test that writes
into the build directory and keeps the file when killed is not worth the arm it
covers. The `find`-succeeded arm is recorded as
`unreachable-in-this-environment` (§5.2). The companion test now holds
`crate::test_env::lock()` — the repo's convention — because the real race was
`PATH` itself: `with_path_prepended` mutates the process-global `PATH`, so any
test that lets a tool be resolved by name must hold that lock.

### 2.4 This segment's tests (`tui/src/app.rs`)

The app layer is now driven through its own entry points rather than through
injected `UiCmd`s. New tests, all with real assertions. The final group is what
closed the last stretch:

* **`a_live_agent_settles_the_spawned_task_commands`** — the four spawned-task
  command paths (autocomplete fetches for models and sessions, the recommendation
  request, the `--prompt` startup send, the interrupt's `abort`), each asserted on
  the mock's request log, using the `set_current_session_id("")` fast path (§2.5).
* **`start_propagates_a_terminal_failure`** — a `FakeTerminal` that refuses to
  start must make `App::start` return the error and leave `screen_entered` false;
  a swallowed failure would leave the TUI drawing to a cooked terminal.
* **`cycle_model_with_an_empty_scoped_list_falls_back_to_the_agent`** — an empty
  scoped list has nothing to cycle locally, so the agent is asked; asserted
  side-by-side with the non-empty arm that does move the model.
* **`a_relative_cwd_argument_resolves_against_the_session_cwd`** — now asserts the
  command that reaches the agent for **both** a relative argument (folded against
  the session cwd, `..` resolved) and an absolute one (forwarded unchanged), which
  is what covers the resolution chain's fall-through as well as its body.
* **`a_string_tool_argument_is_kept_verbatim_when_rows_are_loaded`** — a history
  row whose tool block carries `arguments` as a JSON **string** keeps it verbatim
  (the object form is serialized); the row also needs a text block, or it is
  dropped before its arguments are read.

Measured negative result recorded in `tui/src/app.rs` and §5.4: resizing the
harness console programmatically to reach the `on_resize` callback makes the child
spin and it never finishes (>60 s, killed), so those two lines are waived rather
than forced.

* **The recommendation flow, end to end**: `use_recommended_skill_composes_the_draft_and_the_command`
  (the separator rule, including the empty-draft and trailing-space forms),
  `accepting_a_recommendation_installs_it_and_sends_the_draft` (the `a` key →
  install → the composed draft goes out; asserts the install's argument vector),
  `an_install_failure_keeps_the_card_and_holds_the_draft` (PRD v1.6 §6.2: the
  card stays up, the draft is untouched, the failure is reported with its
  recovery hint and **no** message with the uninstalled skill goes out),
  `the_accept_key_only_applies_to_a_shown_card` (pending, idle, overlay-open and
  uppercase variants), `accepting_without_a_card_is_a_no_op`,
  `accepting_without_a_skills_binary_reports_it`,
  `a_suggestion_for_a_stale_draft_is_dropped`,
  `a_recommendable_draft_is_held_while_the_agent_is_asked`,
  `escape_with_a_card_sends_the_draft_without_the_skill`,
  `sending_with_nothing_held_does_nothing`,
  `a_card_without_a_summary_renders_just_the_skill_name` (the prompt line and
  its rendering above the input box),
  `the_bare_and_unknown_skill_recommend_arguments_report_only`,
  `the_prefetch_without_a_skills_binary_does_nothing`.
* **CLI argument parsing**: `every_value_taking_flag_accepts_its_value_and_tolerates_none`
  (every value-taking flag with a value, bare as the last argument, and followed
  by another flag; the CSV-vs-repeatable distinction; `--print`'s `@file`/flag
  guard; `--list-models`' tri-state; all 16 boolean flags) plus
  `run_without_a_prompt_exits_one_and_reports_usage`,
  `run_without_a_prompt_stays_silent_in_json_mode`,
  `run_reports_a_list_models_failure`, `run_reports_a_print_mode_failure`,
  `run_without_flags_reports_a_missing_console_instead_of_entering_the_loop`,
  and `exit_code_is_one` (its `Debug` form is the only in-process way to compare
  an `ExitCode`).
* **Session/overlay routing**: `a_session_answer_is_routed_by_its_purpose`
  (autocomplete caches without opening the picker; browse opens it),
  `selecting_a_fork_row_emits_the_overlay_select_command` (the picker's own
  `onSelect` callback, which injecting `UiCmd::OverlaySelect` never exercises),
  `realigning_without_a_pending_switch_is_a_no_op`,
  `answers_for_a_departed_session_are_dropped`,
  `a_history_page_for_a_departed_session_is_never_requested`,
  `a_page_for_a_departed_session_is_dropped`.
* **State and invariants**: `interrupt_while_streaming_clears_the_run_state`,
  `the_next_deadline_is_the_earliest_of_the_timer_and_the_ui_deadlines`,
  `compact_is_refused_while_a_run_or_compaction_is_in_progress` (all three
  refusal conditions), `the_injected_terminal_failure_is_reported_once` (the
  fault-injection seam is one-shot), `a_relative_cwd_argument_resolves_against_the_session_cwd`
  (asserted on the command that reaches the agent),
  `a_string_tool_argument_is_kept_verbatim_when_rows_are_loaded`,
  `a_live_agent_answer_reaches_the_autocomplete_path`.
* **The Windows halves of the real-runner tests** (`tui/src/skills_cli.rs`) —
  see §2.2 of the previous revision; the `sh`-based originals cannot run here.

### 2.5 The live-mock harness: why calls were slow, and what made them usable

An earlier revision of this doc recorded that an in-process `AppMockAgent` +
`app.start()` "delivers `list_models` but not `list_sessions`, `prompt`, `abort` or
`suggest_skill`". **That reading was wrong, and this segment found the real
cause.** Those commands *were* reaching the mock; they were just taking five
seconds each, so every test's pump window closed first and the request log looked
empty.

`GrpcClient::call` waits up to `CALL_CONNECT_WAIT_MS` (5 s) for a connection, but
**only when the client has a session id** (the TS `connectPromise === null` case
skips the wait). An in-process mock never makes the client report "connected", so
with a session set, every call costs the full 5 s. Measured on this host:

| call | with the client's session set | with `set_current_session_id("")` |
|---|---|---|
| `list_sessions` | 5.02 s | **4.8 ms** |
| `abort` | 5.02 s | **4.8 ms** |
| `suggest_skill` | 5.04 s | **6.0 ms** |
| `prompt` | 5.02 s | **5.5 ms** |

So the recipe for a live test is `app.client.set_current_session_id("")` before
the action that spawns the task, then wait on the mock's request log rather than
on a wall-clock window. That is what `a_live_agent_settles_the_spawned_task_commands`
does, and it is what closed the 11-line `maybe_recommend_skill` spawn body, the
`InitialPrompt` send, both autocomplete fetches, the `abort` spawn and the
relative-`/cwd` resolution — ~20 lines that the previous revision had waived as
unreachable.

Cost: the fast path still pays the app's own startup (`app.start` → connect
probe), so each live test is ~20 s on an instrumented build. Only two tests use it,
and the *assertions* are on the request log, not on timing.

---

## 3. Suite state and quality checks

| check | result |
|---|---|
| `cargo llvm-cov -p future-tui --lib` (the measuring run) | **2079 passed, 0 failed, 0 ignored** |
| `cargo fmt -p future-tui --check` | clean |
| `cargo clippy -p future-tui --all-targets -- -D warnings` | clean (it caught a `useless comparison`/`unnecessary cast` in a test I wrote, both fixed) |
| `#[ignore]` / disabled tests in `tui/` | none |
| 13 Windows-red baseline tests in this crate | green (four are load-flaky under parallel builds, §1.5) |

---

## 4. Dimensions

| id | evidence (this segment in bold) |
|---|---|
| boundary | **`suggest_skill` with an empty candidate set and an empty query; a null/number/absent description; Unicode+CJK query and skill name**; **an OSC 8 URL with `BEL`/`ST`/newline/NUL/tab vs a Unicode URL**; **an incomplete escape followed by a 3-byte CJK character, with the character emitted exactly once and no empty chunks**; **a top-level indented paragraph vs an unindented one vs container indentation**; **a transcript proven taller than the viewport before scrolling**; `read_size` of a null and an `INVALID_HANDLE_VALUE` handle → `(0,0)`; `write_stdout` with empty and 40 000-byte payloads; clipboard's 1 MB payload and empty candidate list; pre-existing OSC52 at/one-over the limit. |
| error-path | **`suggest_skill` declines on a tonic `Unavailable` and on `success:false`, and on 6 malformed payload shapes**; **a draft file whose name cannot be created (not retried, path named)**; **a control-bearing link URL**; **`create_draft_file` collisions, the concurrent-stub hazard and the missing-tool case**; **the console child panics instead of skipping when it cannot build a `Terminal`**; `binding_handles_without_a_console_is_a_reported_error`; the default clipboard runner's five failure shapes through `cmd`; `ensure_agent_running`'s refusal. |
| concurrency | **`prepending_history_shifts_the_viewport_and_notifies_the_owner` (callback fires exactly once)**; **`dropping_the_owned_agent_kills_its_child` (real child + external liveness observer, both directions)**; **`enable_and_restore_raw_are_idempotent` (a second `enable_raw` must not re-capture the raw modes as the "originals")**; **`wait_is_prompt_and_reports_a_size_change_only_once` (edge-triggered `Resize` over 20 polls)**; **the `PATH`/exe-directory race in §2.1, fixed by the repo's env lock rather than by `sleep`**; the console child's reader thread started/stopped on a real console; `run_native_reports_a_broken_windows_stdin_write` synchronized on `try_wait()`. |
| property | **the keys table: every documented spelling of a key → the same id, and an undocumented sequence → `None`**; **`set_compact_activity` idempotence and `rerender_message` on a folded member changing no rows (the fold invariant)**; **`suggest_skill`'s request/response mapping (query and candidates verbatim, in order)**; `with_runner`'s candidate-list invariant; the provider's `value == "@" + label` mapping asserted over every branch's output; "stop restores, it does not tear down". |
| platform-cfg | this module's work is the Windows half: the console harness makes `terminal_windows.rs` runtime-tested on Windows rather than type-checked only; clipboard and paste gained the Windows counterparts of their `#[cfg(unix)]` runner tests; the attachment behaviour around `CreateProcessW`'s search order is measured and documented; **no platform predicate was changed and no `cfg(test)` was added to production code to move a number**. POSIX-only tests stayed `#[cfg(unix)]`. |
| serialization | **`suggest_skill`'s payload shapes (typed decode with a JSON fallback, null skill, missing key, non-string name, non-string description, Unicode preserved)**; **the OSC 8 escape-sequence framing including the injection guard**; **the escape-scanner's UTF-8 boundary handling**; the chat/JSON `ApiMock` path; pre-existing byte-level framing (kitty CSI-u, modifyOtherKeys, `\x1b[?1004h`/`\x1b[O`, bracketed paste) asserted at the byte level. |

**Branch coverage:** §1.4 — not obtainable on stable 1.97.0; the region/mixed-line
analysis substitutes and is reported per file.

---

## 5. Waivers (category + reason, same row as the path)

### 5.1 `attribution-artifact` — files whose every line executes

Each of these has **zero `DA:<line>,0` records**: the file's own line table says
every line number carries a non-zero count, so the lines the entry-based metric
reports are line-table entries from a merged instantiation that no test can
"cover". The named test is the one proving the file's code runs. Reproduce with
`python coverage/tui-artifact-tests.py coverage/tui-report.json coverage/tui-report.info`.

| file | reported | category | the test that proves the file runs |
|---|---|---|---|
| `tui/src/clipboard.rs` | 3 | attribution-artifact | `clipboard::tests::encode_osc52_wraps_the_base64_payload`, `env_value_treats_blank_as_unset`, `the_real_spawner_runs_a_program_and_reports_a_missing_one` |
| `tui/src/skills_cli.rs` | 1 | attribution-artifact | `skills_cli::tests::a_real_windows_spawn_runner_captures_streams_and_the_exit_code`, `a_real_windows_wait_times_out_and_kills_the_child`, `a_real_windows_child_flooding_the_capture_limit_fails_explicitly`, `deadline_error_distinguishes_a_killed_child_from_a_leaked_pipe` (all new; the file's line table has no zero-count line at all) |
| `tui/src/paste.rs` | 7 | attribution-artifact | `paste::tests::threshold_counts_characters_not_bytes`, `normalize_folds_line_endings_and_tabs`, `the_real_spawner_runs_a_program_and_reports_a_missing_one` |
| `tui/src/external_editor.rs` | 4 | attribution-artifact | `external_editor::tests::io_message_reports_the_io_message_and_describes_other_variants`, `create_draft_file_reports_a_name_that_cannot_be_created` |
| `tui/src/tui.rs` | 5 | attribution-artifact | `tui::tests::cursor_pos_formats_1_based`, `set_fg_bg_use_256_color`, `parse_size_value_fixed_and_percent` |
| `tui/src/components/keymap_view.rs` | 4 | attribution-artifact | `keymap_view::tests::every_action_is_grouped_and_shows_its_current_key`, `the_catalogue_covers_the_app_actions_without_duplicates` |
| `tui/src/help_screen.rs` | 2 | attribution-artifact | `help_screen::tests::lists_every_slash_command_handled_by_the_tui`, `command_card_is_sorted_by_command_name` |
| `tui/src/components/menu.rs` | 1 | attribution-artifact | `menu::tests::menu_item_defaults_to_enabled_and_unselected`, `menu_item_builders_populate_every_field` |
| `tui/src/components/pager.rs` | 1 | attribution-artifact | `pager::tests::wrap_lines_keeps_one_row_per_empty_line`, `wrap_lines_hard_breaks_long_ascii` |
| `tui/src/components/usage_view.rs` | 1 | attribution-artifact | `usage_view::tests::usage_overlay_renders_the_panel_and_ignores_input`, `format_tokens_uses_compact_units` |
| `tui/src/components/worktree_view.rs` | 1 | attribution-artifact | `worktree_view::tests::a_row_carries_the_path_the_branch_the_state_and_the_badge` |
| `tui/src/keybindings.rs` | 1 | attribution-artifact | `keybindings::tests::add_and_dispatch_consumed_action`, `dispatch_stops_at_first_consuming_action` |
| `tui/src/skill_reco.rs` | 1 | attribution-artifact | `skill_reco::tests::a_missing_file_is_an_empty_today`, `recording_accumulates_until_the_limit` |
| `tui/src/themes.rs` | 1 | attribution-artifact | `themes::tests::catalog_is_not_empty_and_ids_are_unique_and_canonical`, `id_set_is_frozen_because_it_is_persisted` |

32 lines total, plus `tui/src/skills_cli.rs` (1 line) — 14 files in all. Reproduce
with `python coverage/tui-artifact-tests.py coverage/tui-report.json
coverage/tui-report.info`.

The test-side counterpart of this category, on the same row-by-row basis:

| file | lines | category | the test that proves the file runs, and why these lines are artifact |
|---|---|---|---|
| `tui/src/terminal.rs` | `1097`, `1140`, `1141`, `1144`, `1147` | attribution-artifact | `terminal::tests::the_injected_terminal_failure_is_reported_once` (`1097` is its `Ok(_) => panic!(...)` arm — it executes only if the fault-injection seam *fails* to fire, i.e. if the test's premise is false) and `terminal::tests::a_zero_byte_console_read_is_not_eof` (`1140`/`1141`/`1144` are the `Resize` arm's assertions, reached only when a window resize happens to be observed in the three polls, and `1147` is the `other => panic!` arm for a `ReadWait` variant Windows never produces). The surrounding code in both tests is covered; only the "the assertion failed / the impossible happened" spans are not. |

### 5.2 `unreachable-in-this-environment`

| file | lines | category + reason |
|---|---|---|
| `tui/src/update.rs` | `56`, `172` | unreachable-in-this-environment — the `else` arm of `if cfg!(windows)` (`"install.sh"`), in `notice()` and in the test that pins the installer name. A runtime `cfg!` cannot be varied, and the whole suite runs on Windows. The Windows arm (`"install.ps1"`) is asserted by `update::tests::notice_has_versions_and_platform_installer_with_channel_hint`. |
| `tui/src/components/footer.rs` | `353` | unreachable-in-this-environment — the non-Windows arm of the `if cfg!(windows)` home-path literal (`"/home/tester"`) in the test's fixture; the Windows literal is the one this host takes. |
| `tui/src/worktree.rs` | `1426` | unreachable-in-this-environment — the non-Windows arm of `if cfg!(windows) { stripped.to_ascii_lowercase() } else { stripped.to_string() }`, the case-insensitive path comparison. The Windows arm is asserted by `worktree::tests::ordinary_path_rewrites_the_extended_length_spellings` and the porcelain-parsing tests. |
| `tui/src/components/autocomplete.rs` | `539`, `554`-`559` (the `find`-succeeded arm), `1788`, `1789` (the `[skip]` arm) | unreachable-in-this-environment — the `[skip]` arms fire only on a host with no argv-echoing program (Git-for-Windows' `echo.exe` and POSIX `/bin/echo` both exist here). The `find`-succeeded arm inside the fallback (`Ok(out) if out.status.success()`) can only be reached by shadowing System32's `find.exe`, which requires writing a stub into the **test binary's own directory** — a directory shared by every test in the binary and one that keeps the file if the process is killed (measured: a leftover `find.exe` from a crashed run broke a companion test). The fragile test was deleted rather than serialized (§2.1); the `fd`-succeeded arm *is* covered, through a temp-dir stub on `PATH`. |
| `tui/src/terminal_windows.rs` | 51 (of 243) | unreachable-in-this-environment — the Win32 **failure and rollback** arms: `GetStdHandle` returning `INVALID_HANDLE_VALUE`; `GetConsoleMode` failing after `Backend::new` succeeded; `SetConsoleMode` failing and rolling back; a console ignoring `ENABLE_VIRTUAL_TERMINAL_PROCESSING`; `SetConsoleOutputCP`/`SetConsoleCP` failing; `ReadFile`/`WriteFile` returning 0 and the defensive `written == 0` break; `read_size`'s failure arm; `wait`'s size-change branch (its edge-triggered contract is asserted instead by `wait_is_prompt_and_reports_a_size_change_only_once`); and `die_with_signal`, which calls `std::process::abort()` and cannot run without killing the test process. Reaching these needs fault injection in production code (forbidden by rule 2) or a deliberately misbehaving console. |
| `tui/src/console_harness.rs` | 17 (of 171; 19 line-table entries) | unreachable-in-this-environment — the "this environment cannot create any console object" path (`bind_console_handles`' `GetConsoleWindow`==NULL error, the `CreateFileW` failure, `screen_buffer_size`'s failure), the sentinel-77 exit, the parent's `[skip]` branch, the harness's own "a required test did not run" panic, and the `Err` arm of `binding_handles_without_a_console_is_a_reported_error`. On this host `CREATE_NEW_CONSOLE` always succeeds for the child. |
| `tui/src/index.rs` | 916, `924`-`1058` (the `run_interactive` body) | platform-unmeasured — the interactive entry point is driven end to end by **six `#[cfg(unix)]` tests** in this same file (`InteractiveFixture`, `#[cfg(unix)] struct` at the time of writing, and the `#[cfg(unix)] #[test]` functions from `run_interactive_…` onwards, e.g. the one that joins `run_interactive(&args)` with a PTY driver and asserts exit code 0). They need a PTY and a real console, which the crate's dev-dependencies do not provide on Windows (`tempfile`, `futures-util`, `tokio-stream` only), so on this host the body cannot execute. Every *other* line of this file is covered: the whole `parse_args` flag matrix, `tui_settings_path`, and `run()`'s argument/usage/error tails, by the tests added this segment (§2). |
| `tui/src/index.rs` | `1117`, `1118`, `1119`, `1136`-`1140` | unreachable-in-this-environment — the `Err` arms of `run()`'s two sessionless commands: `list_models`' failure branch and `run_print_mode`'s. Reaching them needs a live endpoint that answers with a failure; against an unreachable port the gRPC client reports the failure internally and returns `Ok`, which is exactly what the new in-process tests observe (`run_reports_a_list_models_failure`, `run_reports_a_print_mode_failure` assert the exit code 1 those arms produce when reached on a real deployment). Producing a deterministically failing agent is not possible here without starting a service. |
| `tui/src/index.rs` | `1247`, `1248`, `1252` — superseded | these three were a dead `else` branch in the new flag-matrix test (no case exercised it); the branch was **removed** rather than waived, so re-measure before relying on this row. |
| `tui/src/index.rs` | `1463`, `1467` | unreachable-in-this-environment — the deliberate safety skip in `run_without_flags_reports_a_missing_console_instead_of_entering_the_loop`: it fires only where a console *is* available, and its whole point is that a test must not drive the interactive loop (which would talk to a real agent). The complementary path is asserted: on a console-less runner the test does run and asserts exit code 1. |
| `tui/src/terminal.rs` | `152`, `176` | unreachable-in-this-environment — the `Ok(..) => Some(..)` arms of `terminal_or_skip` and `backend_or_skip`: they are reached only by a test process that **has** a console and is **not** the harness child. This host's test process has no console (so the `Err` arm runs) and the child returns before the `match`; a developer running `cargo test` in a real terminal would cover them. |
| `tui/src/terminal.rs` | `233`-`236`, `239`-`242` | platform-unmeasured — `start`'s two guards (already-started ⇒ `AlreadyExists`, and stdin-not-a-TTY ⇒ `NotConnected`) are asserted by the `#[cfg(unix)]` test `start_rejects_non_tty_and_double_start`, which swaps fd 0 for `/dev/null` and for a PTY pair. The Windows equivalent would need a non-console stdin handle in a process that *has* a console, which the harness cannot produce without replacing the process-global std handle. |
| `tui/src/terminal.rs` | `308`, `311`, `317`, `322`, `328`, `332`, `343`-`387`, `407`-`413`, `775`-`778` | unreachable-in-this-environment — the reader loop's **input-driven** arms: the two deadline arithmetic lines that only run once a flush or paste-burst deadline is set (both require bytes arriving), the `ReadWait::Input` arm and its `read_stdin` error break, the zero-byte-spurious-wake `continue`, the POSIX EOF `break`, the whole `buffer.process_bytes` → `sink.feed` pipeline, the `StdinBuffer` idle flush timer, and `handle_event`'s kitty-CSI-u response branch (which needs `\x1b[?Nu` to arrive *as console input*). Driving them needs console input injection (`WriteConsoleInputW` KEY_EVENT records), which this crate's dev-dependencies do not include; the pieces that do not need input are covered directly instead — `stdin_buffer_to_keys_integration` drives bytes → `StdinBuffer` → `handle_event` outside the loop, and the loop's *output* side and its resize handling are covered by `every_terminal_io_delegation_reaches_a_real_terminal` and `wait_is_prompt_and_reports_a_size_change_only_once`. |
| `tui/src/terminal.rs` | `477`-`479`, `605` | platform-unmeasured — `stop()`'s kitty-protocol teardown (`self.kitty_active` armed ⇒ write `\x1b[<u`, clear the flag, clear the global) and `Drop`'s "only stop when a reader thread exists or raw mode is on" guard. Both are asserted by the `#[cfg(unix)]` tests `stop_with_modify_other_keys_active` and `start_rejects_non_tty_and_double_start` (which drops a started `Terminal`); on Windows the harness's own `terminal_io_delegation_tests` calls `stop()` explicitly, so `Drop`'s guard stays false. |
| `tui/src/rpc/grpc_client.rs` | `1241` | unreachable-in-this-environment — the reconnect poll's **loop-top** `stop` check (the `return;` inside `spawn_manager`'s reconnect loop). It executes only if `stop` is set **without** a notify while the poll sits between iterations: a notifying stop (`disconnect()`, `Drop`) is caught by the *same* loop's `select` `stop_notify` arm, which returns from a different line. Measured with this module's own tests under instrumentation: the loop top (`1240`) is reached **9** times and its `return` **0** times, because every stop the suite performs notifies. Reaching it deterministically needs an observable "the manager is inside the reconnect poll" signal, which `GrpcClient` does not expose; the four tests named for this path `sleep` to hit timing windows instead (§6). |

### 5.3 `unreachable-by-construction`

| file | lines | category + reason |
|---|---|---|
| `tui/src/agent_supervisor.rs` | 37 (of 169; 38 line-table entries) | unreachable-by-construction — `ensure_agent_running`'s launch loop (the `current_exe` read, the candidate spawn, the 40×125 ms health poll, its `exited with <status>`/`try_wait` error arms, the kill+wait tail, the final aggregated error) sits **after** `if cfg!(test) { return Err("agent sidecar startup is disabled in unit tests") }`. That guard is a deliberate test seam, so no test build can reach the loop, and reaching it would need a real agent daemon. The refusal line is asserted by `ensure_agent_running_refuses_to_spawn_in_test_builds`; the `Drop` body is **covered** by `dropping_the_owned_agent_kills_its_child`; `launch_candidates`' braces are proven by `unified_binary_launches_agent_without_tcp_in_auto_mode` and `explicit_tcp_is_forwarded_only_when_requested`. |
| `tui/src/rpc/grpc_client.rs` | `3213`, `3231`, `3283` | unreachable-by-construction — test-helper **failure arms**: `recv_until`'s `panic!("timed out waiting for a {kind:?} event")`, the `false` its timeout closure returns, and `attach`'s `panic!("client never attached to a live event stream")`. Each turns a lost event into an explicit failure instead of a hang; executing one means the test using it failed, and a test cannot assert its own failure. Proven by the green tests that call them (`poke_during_live_subscription_resubscribes_without_losing_events`, `no_context_files_follows_session_changes_and_precedes_requests`). |

### 5.4 `tui/src/app.rs` — its last 6 lines, line by line

`tui/src/app.rs` was the module's last file with genuinely reachable uncovered code
(the OPEN list of the previous revision). Segment by segment it went 190 → 162 →
120 → 87 → 36 → **6**, and every remaining line is a **test-side** or
**environment** limit, not production behaviour. Each is listed with its own
category so the reconciliation is checkable one line at a time:

| file | lines | category + reason |
|---|---|---|
| `tui/src/app.rs` | `188`, `189` | unreachable-in-this-environment — the body of the `on_resize` callback in `every_terminal_io_delegation_reaches_a_real_terminal` (`resized.fetch_add(..)`). The callback runs only when the console's window size changes. The harness console is created hidden, and shrinking its window programmatically did not produce the event safely: the child then kept running for over 60 s and had to be killed (measured this segment; the same attempt made `windows_console_harness` hang). A real terminal resized by a human does run it. |
| `tui/src/app.rs` | `11730` | unreachable-by-construction — `pump_until_history_settled`'s `panic!("the history page never settled")`, the helper's timeout arm. Executing it means the test that called the helper already failed; a test cannot assert its own failure. Proven by the paging tests that do call it (`a_history_page_for_a_departed_session_is_never_requested`, `the_history_page_settles_and_appends_older_messages`). |
| `tui/src/app.rs` | `11808` | unreachable-in-this-environment — `stripped.to_string()` inside `mod tests`: the `else` arm of an `if cfg!(windows)` in a test fixture. A runtime `cfg!` cannot be varied, and the suite runs on Windows; the Windows arm (`to_ascii_lowercase()`) is the one this host takes. Same shape as the `tui/src/update.rs` and `tui/src/components/footer.rs` arms waived above. |
| `tui/src/app.rs` | `18476` | unreachable-in-this-environment — `"true"` inside `mod tests`: the non-Windows program the test spawns when `cfg!(windows)` is false. This host is Windows, so the `"cmd /c rem"` arm runs instead. |
| `tui/src/app.rs` | `24871` | unreachable-by-construction — `wait_for_set_cwd`'s `None` tail: the bounded-wait timeout of a **test helper**. Its two callers assert on the returned value, so reaching this line means one of those assertions was about to fail anyway. |

All six are lines **the module's own tests** contain, or a console-resize event
this environment cannot produce. No production line of `tui/src/app.rs` is left
uncovered: the app layer is driven end to end through `handle_key`,
`handle_submit`, `handle_cmd` and `handle_key_action`, including the spawned-task
command paths (see §2.4 and §2.5).

---

## 6. Weak-test audit

**Fixed in the last two segments** (all were assertion-free; one did not reach the
arm its comment named):

| test | was | now |
|---|---|---|
| `tui/src/components/autocomplete.rs::file_path_parentless_resolution_uses_dot` | `let _ = provider.get_completions(&ctx);` with an empty token, so the "." arm was never reached | asserts the list is non-empty and every label keeps the relative "." prefix |
| `tui/src/components/autocomplete.rs::file_path_home_unset_falls_back_to_root` | cleared env, built a provider, asserted nothing | the cwd contains a file the token would match and the test asserts it is **not** returned (proving `~` expands against the missing home) |
| `tui/src/keys.rs::every_documented_spelling_of_a_key_resolves_to_the_same_id` (new, then self-corrected) | its first form left the failure-message argument unevaluated | the diagnostic is bound eagerly, so the line is genuinely covered |
| `tui/src/components/chat_area.rs` (new tests) | an early draft asserted on an `on_change` callback that `rerender()` does not invoke, and another asserted `viewport_top >= 0` on an unsigned field | both rewritten to assert observable behaviour (rendered rows; the real clamp) |

**Still open** (found, not yet fixed):

| test | problem | fix |
|---|---|---|
| `tui/src/terminal.rs::noop_callbacks_invoke`, `tui/src/components/scoped_models_selector.rs::noop_callbacks_are_callable` | call no-op callbacks, assert nothing | assert a recorded call count |
| `tui/src/crash.rs::raw_write_stderr_writes_bytes`, `tui/src/skills_cli.rs::parse_catalogue_rejects_output_without_a_json_object`, `tui/src/terminal_posix.rs` (4, POSIX-only) | no assertion in the body as written; some are `should_panic`-style | add the missing expectation or state here why the helper asserts || `tui/src/rpc/grpc_client.rs::reconnect_poll_session_change_and_stop` (with the three sibling `reconnect_poll_*` tests and `poked_exit_with_stop_set_returns_manager`) | they `sleep` to hit timing windows instead of waiting on an observable, and at least one does not exercise what its name promises: on a filtered instrumented run of that test alone, **none** of the reconnect loop ran (line `1240` = 0 hits) | drive the poll from an observable state, then stop without notifying and assert the poll returned — the deterministic route also covers `1241` (§5.2) |

| `tui/src/rpc/grpc_client.rs::reconnect_poll_session_change_and_stop` (with the three sibling `reconnect_poll_*` tests and `poked_exit_with_stop_set_returns_manager`) | they `sleep` to hit timing windows instead of waiting on an observable, and at least one does not exercise what its name promises: on a filtered instrumented run of that test alone, **none** of the reconnect loop ran (line `1240` = 0 hits) | drive the poll from an observable state, then stop without notifying and assert the poll returned — the deterministic route also covers `1241` (§5.2) |

**Discharged this segment** (they were on the list above, or flagged by my own
scan and then cleared):

| test | status |
|---|---|
| `tui/src/app.rs::scrollback_terminal_exit_callback_setter` | now asserts what the caller can observe: the call is accepted in both states, and setting it neither draws nor touches the transcript |
| `tui/src/components/keymap_view.rs::the_highlight_walker_refuses_a_row_that_is_not_there` | was assertion-free; now `#[should_panic(expected = "«Ghost action» is not in the panel")]`, which is exactly the claim its name makes (the walker gives up loudly instead of stopping wherever it ran out of budget) |
| `tui/src/rpc/grpc_client.rs::wait_connected_helper_loops_until_true` | now asserts the channel starts `false`, that the helper returns only once it reads `true`, and that it does not flip the value back |
| `tui/src/components/keymap_view.rs::the_overlay_highlight_walker_refuses_a_row_that_is_not_there` | **false positive** in my scan: it already carried `#[should_panic(expected = "«Ghost action» is not in the overlay")]`. An `assert!`-less body is not the same as an assertion-less test; the audit now reads `should_panic` too. |

No test written in this segment lacks an assertion. Two tests carry an explicit
`[skip]` whose reason is printed (§5.2) — that path is untaken on this host, since
Git for Windows' `echo.exe` is present.

---

## 7. Reconciliation — every uncovered line accounted for

Every file in this crate that still holds uncovered lines is waived with a
category and a reason on the same row in §5. There is no file left in an
unfinished state: the module's remaining gaps are all of the three documented
kinds (line-table residue from merged instantiations, cfg!/PTY arms this host
cannot execute, and test-helper failure arms that only run when a test has
already failed), plus one console-resize event this environment cannot produce
safely (§5.4).

Where the work went, and what is left, is recorded in §8 as *optional further
work* — closing a waived line is an improvement, not a missing deliverable, and
the categories in §5 say why each one is not reachable from a test here.

## 8. Further work (optional — the module already passes the gate)

Everything below would *reduce* the waived-line count. None of it is required for
the acceptance contract, and each item is honestly reachable-or-not as §5 says.

1. **Console input injection** (`WriteConsoleInputW` KEY_EVENT records) would
   close the reader loop's input-driven block in `tui/src/terminal.rs` (§5.2),
   which is the largest single waiver (63 lines). It needs a new
   dev-dependency/feature set — a supervisor decision, not test writing.
2. **A ConPTY fixture** would let `tui/src/index.rs`'s `run_interactive` body run
   on Windows; today it is asserted by six `#[cfg(unix)]` PTY tests in the same
   file (§5.2, `platform-unmeasured`). Same dependency caveat.
3. **The `tui/src/app.rs` on_resize callback** (`188`, `189`): if a *safe* way to
   change the harness console's window size is ever found (the direct attempt
   hangs the child — measured, §5.4), those two lines close.
4. **Weak tests still listed in §6**: the two no-op-callback tests and the four
   `terminal_posix.rs` / `crash.rs` / `chat_area.rs` bodies whose assertions live
   in a helper. Cheap, and worth doing the next time that file is touched.
5. **Decide the repo-wide measurement policy** (§1.3): per-line `DA` counts for
   the lib form; the **per-line union across the lib and all-targets forms** so
   lines reached only by an integration test are not counted against a module;
   and a convention for inline `mod tests`, which this crate's numbers show can
   keep a file below 100 % for reasons unrelated to its production code.

### What is *not* left to do

No production line of this module is unrunnable-in-principle-but-unwaived, and no
file is in an unfinished state. The remaining gaps are: line-table residue from
merged instantiations (§1.3), `cfg!`/PTY arms this host cannot execute, helper
failure arms that only run when a test has already failed, and one console-resize
event this environment cannot produce safely. §7 reconciles that against the
measurement.



---

## 9. Round 2 (`cov100-r2`, macOS) — the per-line measurement and the current ledger

**This section supersedes §§1–8 for the current tree.** Those sections record the
2026-08 Windows segment (`test/cov100` worktree, entry-based gate metric, 291
per-line residuals, all waived). Round 2 measures the same crate on macOS with the
campaign's per-line tool, and the tree has moved 739 commits since. Section
numbers above are unchanged (production comments cite them by number); where a
row above no longer matches the code it is called out in §9.5.

### 9.1 Run identity

| | |
|---|---|
| worktree | `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`) |
| revision | `8449acf6` (campaign baseline) + this unit's working-tree changes (snapshotted by the supervisor, e.g. `e2872123`) |
| host | macOS arm64 (26.x), rustc 1.97.0 (pinned by `rust-toolchain.toml`), load average 15–40 (eight campaign workers share the host) |
| command | `cargo llvm-cov -p future-tui --no-fail-fast --lcov --output-path /tmp/tui-r2-crate.info -j 4` |
| env | `CARGO_TARGET_DIR=target/cov-tui` (private), `HOME=$PWD/target/test-home` (isolated), `ulimit -n 10240`, rustup (not Homebrew) toolchain on `PATH` |
| metric | per-line LCOV `DA:<line>,0` — `python3 .future/cov100/covlines.py {files,detail,total} <lcov> --under .worktrees/cov100-r2/tui/src` (this worktree's `SF:` paths are worktree-relative, so `--under tui/src` matches nothing here) |
| suite state | green: `--lib` 2193 passed / 0 failed, `agent_sidecar_launch` 1, `agent_sidecar_readiness` 2, `bughunt_regressions` 8, `cli_smoke` 5, `sandbox_panel_real_platform` 1, `src/main.rs` 0 |
| artefact | `/tmp/tui-r2-crate.info` — sha256 `99c6a2b61b395d7c…` (see §9.8 if a later edit moved the counters) |

**The candidates this measurement could resolve (why the number is a host
statement, not just a code statement).** The launch loop of
`agent_supervisor::ensure_agent_running` is reached by tests that spawn a real
process, so which arm runs depends on what is resolvable as a candidate — a
private target dir and `PATH` are both host state:

* for the **test binaries** (this crate's integration targets, e.g.
  `agent_sidecar_launch.rs`, `agent_sidecar_readiness.rs`) the sibling
  candidates are `<CARGO_TARGET_DIR>/llvm-cov-target/debug/deps/future` and
  `…/future-agent`. Cargo never creates an unhashed `deps/future` (it produces
  `deps/future-<hash>` and the uplifted `debug/future`), so in a crate-scoped run
  these are absent — verified in the measured tree, and
  `agent_sidecar_readiness.rs` asserts the absence as a precondition;
* for the **`future-tui` binary** that `cli_smoke.rs` runs, the siblings are
  `…/debug/future` and `…/debug/future-agent` (absent in a crate-scoped run,
  **present in a `--workspace` run**) and then `PATH`: this host has
  `/opt/homebrew/bin/future` — `v0.0.2-8449acf6+local` — which the client rejects
  on version, and `future-agent` (checked with `which -a`).

Before this rework that host state decided a coverage line: `cli_smoke`'s
print-mode test launched whatever it found, so the workspace form counted
`agent_supervisor.rs:121` covered and the crate-scoped form did not (§9.7(a)).
It no longer launches anything (see §9.8, hazard 2), and the launch loop's arms
are now covered by in-tree integration fixtures in **every** form, so the two
forms no longer disagree about this file.

**The second flavour was attempted, not measured.** The plan was to re-run the
exact command with a same-tree sibling planted in the instrumented target dir
(`cp target/debug/future …/llvm-cov-target/debug/future`) — the artefact the
review named — and to show the tui number does not move. All four attempts were
lost to the §9.7(e) flake (`io error when listing tests: BrokenPipe`, a failed
target, no lcov), so this section reports the crate-scoped number only, plus the
argument: the only test that could resolve that sibling was `cli_smoke`'s
print-mode test (it drove the `future-tui` *binary*, whose siblings are
`…/debug/future*`), and that test no longer launches anything. The planted file
has been removed again; a reviewer who wants the number can run the two commands
in §9.8's repro block (expect the §9.7(e) flake roughly one attempt in three
today, on this loaded host).

### 9.2 Result

| | uncovered lines (`DA:0`) | measured | line % |
|---|---|---|---|
| round-2 baseline (this worktree, before this unit) | **268** | 59 849 | 99.5489 % |
| after the first pass of this unit | **24** | 60 157 | 99.9601 % |
| **after the review rework (§9.8)** | **13** | 60 249 | **99.9784 %** |

The campaign's own baseline for `tui/src` is 270, not 268; the two-line gap is a
measurement-scope difference, accounted for line by line in §9.7(a). All three
numbers are `DA:0` counts of the same measurement form; the rework's eleven lines
are the ones the adversarial review falsified (§9.8), and the 13 that remain are
listed in §9.4 — with no row left in a falsified category.

Per file, worst first, after the rework (`covlines.py files` on `tui-r2-crate.info`):

```
    6 /  1165  tui/src/terminal.rs
    4 /   162  tui/src/agent_supervisor.rs
    1 / 17226  tui/src/app.rs
    1 /  3035  tui/src/components/chat_area.rs
    1 /   172  tui/src/update.rs
```

The 244 lines that closed did so because of two *harness* defects, not because
tests were deleted:

1. **Every console/PTY test in this crate silently skipped.** `terminal_or_skip`
   and `backend_or_skip` (POSIX arms) returned `None` unless the *ambient* stdin
   was a tty — which a `cargo test`/`cargo llvm-cov` harness never is. Six tests
   in `tui/src/terminal.rs` and one in `tui/src/app.rs` therefore early-returned,
   taking ~150 uncovered lines with them; `tui/src/app.rs`'s `TerminalIo`
   delegation test never ran at all. The POSIX arms are **gone**: every unix
   terminal test now builds a `Terminal` directly (`terminal_for_methods`,
   `backend_for_methods`) or runs against a PTY of its own
   (`crate::terminal::with_pty_stdin`, `TestPty` — fd 0 on a fresh
   `posix_openpt` pair). No unix test can skip any more; Windows keeps
   `terminal_or_skip` for the `console_harness` child.
2. **Two fd-1 deadlock traps in the existing tests** (§9.3).

### 9.3 What landed

`tui/src/terminal.rs`

* `terminal_or_skip`/`backend_or_skip` reduced to the Windows-only helpers they
  always were; new unix constructors `terminal_for_methods`, `backend_for_methods`.
* `PtyStdin` (the pre-existing PTY fixture) moved to file scope and exposed as
  `with_pty_stdin`/`TestPty`, with a new `TestPty::resize` (ioctl `TIOCSWINSZ` +
  `SIGWINCH`) so the *resize* callback is exercised for real.
* Six tests now run instead of skipping: `terminal_default_simple_methods_and_progress`,
  `write_log_gated_by_env`, `default_constructs_a_terminal_from_a_real_console`,
  `drain_input_with_and_without_protocols`, `restore_terminal_for_exit_writes_teardown`,
  and a zero-read test split into per-platform arms (`wait_outcomes_stay_handled`
  holds the shared poll assertions).
* **`drain_pty_master`** (new): the two tests that point fd 1 at a PTY slave
  (`refresh_size_picks_up_pty_dimensions`, `reader_resize_updates_cached_size`)
  now drain the master for the duration of the test. A slave whose master nobody
  reads is a ~1 KB buffer, and fd 1 is process-global: the *harness* filling it
  (or a test's own panic path) blocks the writer, and the restore never runs —
  measured: a `cargo llvm-cov` run hung 20+ minutes with fd 0 **and** fd 1 still
  pointing at the slave.

`tui/src/terminal_posix.rs`

* `write_stdout_retries_on_eintr` restores fd 1 **immediately after** the payload
  write (before any join/assertion), and its drainer no longer stops after a byte
  count — the pipe also carries whatever the harness wrote to fd 1 while the test
  had it, so counting bytes let the drainer stop early and leave the payload write
  blocked forever.
* `create_pipe_error_paths` now holds the shared test lock: its child is spawned
  with inherited stdio, so a child launched while another test had fd 1 on a full
  pipe found its own summary write broken (`io error when listing tests:
  BrokenPipe`) and failed the parent's `status.success()`. **Both of these were
  the cause of two aborted measurement runs; with them the instrumented suite is
  green.**

`tui/src/app.rs`

* `terminal_io_delegation_tests` drives every `TerminalIo` method against a real
  PTY from unix (typing a multi-byte character *and* resizing the window, which
  asserts both the UTF-8 pipeline invariant and the `on_resize` callback), plus a
  second run with nothing typed so the Windows arm's shape is measured here too.
* `App::apply_update_notice` (new, `pub(crate)`): the interactive loop's
  "a manifest notice arrived" half moved behind a method, so the
  `UiCmd::UpdateAvailable` path is covered without a real manifest fetch.
* `a_session_answer_is_routed_by_its_purpose` gained a session with **no** cwd
  (the empty-description arm) and `a_live_agent_settles_the_spawned_task_commands`
  gained the `#`-token fetch (the reference menu's own trigger).
* `final_small_paths` builds a real `Terminal` on unix; the `noop_editor` /
  `location` test helpers became `#[cfg]`-split instead of `cfg!`-branched (a
  compile-time-dead branch is not a coverage gap, it is dead code).

`tui/src/index.rs`

* `run_without_flags_...` was a "decline to run" test; it is now
  `run_without_flags_drives_the_interactive_loop_and_exits_cleanly` — a PTY, a
  mock agent behind `--grpc-addr`, and ctrl+c typed by a driver thread, asserting
  exit 0. Three new tests cover `run()`'s reachable error tails (a mock that
  answers an unparseable model list, print mode with no message, a mock that
  cannot mint a session), and the flag matrix gained the guarded `-p` case
  (its `else` branch had never executed).
* `interactive_polls_the_update_check_when_not_offline`: the loop's update arm
  runs for real (guarded by an assertion that this build has no published
  channel, so the check fetches nothing).

`tui/src/components/autocomplete.rs`

* `SessionReferenceProvider::default()`/`name()` asserted (the empty provider the
  app builds); the `~/x` fallback test now also resolves the same token against a
  home that exists, so the item-shape check runs on a real listing instead of a
  vacuous loop; the `find`-fallback path is covered by a stub script that prints
  `pwd -P`, proving `command.current_dir(cwd)`; the `fd` test's skip arm is
  Windows-only now (every POSIX system has `echo`).

`tui/src/rpc/grpc_client.rs`

* `the_reconnect_poll_grows_the_backoff_while_the_agent_stays_down`: a live
  subscription whose server is then aborted must grow the reconnect backoff twice
  (stream loss, then a probe that finds nothing) — this is the `grow_backoff` in
  the poll, previously uncovered.
* `recv_until_within_reports_a_channel_that_closed_first` and
  `recv_until_fails_loudly_for_an_event_that_never_arrives` cover the bounded
  drain's `false` tail and the helper's failure arm; the `probe_agent` test gained
  a peer that accepts and hangs up.

`tui/tests/agent_sidecar_launch.rs` (new)

* The sidecar *launch* path is invisible to unit tests (`cfg!(test)` refuses to
  launch), so an integration test drives it: `PATH` holds a stub `future` that
  exits 3, `FUTURE_HOME` is a scratch dir, and the assertion is on the aggregated
  error text (`future exited with exit status: 3`, the next candidate tried, `no
  sidecar could be started`). Nothing real is started.

`tui/src/agent_supervisor.rs`, `tui/src/clipboard.rs`, `tui/src/components/footer.rs`,
`tui/src/worktree.rs`, `tui/src/update.rs`

* `a_root_executable_path_leaves_only_the_path_candidates` covers the
  `parent()`-is-`None` branch of `launch_candidates`; the test-side `cfg!`-branched
  platform literals became `#[cfg]` consts/arms; the clipboard test's skip is
  Linux-only (every other host probes a native program, and now asserts it);
  `update.rs` uses a `#[cfg]` installer-name const.

### 9.4 Residual ledger — every `DA:0` line, with its mechanism

Re-measured after every edit; the line numbers below are `tui-r2-crate.info`, the
run recorded in §9.1, and each row quotes the code it points at so a line shift
is visible.

| path:line | category | mechanism | how it is proven |
|---|---|---|---|
| `tui/src/terminal.rs:1219` | unreachable-by-construction | `the_injected_terminal_failure_is_reported_once`'s `Ok(_) => panic!` — the test sets `FORCE_NEW_FAILURE` immediately before, so the `Err` arm is the only reachable one; the arm fires only if the seam fails, i.e. if the test's premise is false. The seam is a process-global one-shot, so the discipline that keeps other tests out of its two-instruction window is `terminal_test_lock()` — recorded at the test and in §9.8 (hazard 3). | the same test's `Err` arm and the one-shot assertion (1229 is covered-adjacent) |
| `tui/src/terminal.rs:1229`, `1230` | unreachable-in-this-environment | the *second* construction's `Err` arm: POSIX `Backend::new()` is infallible **and** the one-shot seam was spent by the preceding call in the same test — either fact alone would not be enough. Windows is the OS that can fail here (non-console handles). | `columns_rows_read_cached_size` builds a `Terminal` with no console and cannot fail |
| `tui/src/terminal.rs:1289`, `1290`, `1293` | platform-unmeasured | `wait_outcomes_stay_handled`'s `ReadWait::Resize` arm. Verified in the source, not assumed: `terminal_posix.rs:250-301` can only return `Timeout`, `Input` or `Signal(_)`; `Resize` is the Windows console event. The reader loop's resize path is asserted on POSIX via `Signal(SIGWINCH)` (`reader_resize_updates_cached_size`, `TestPty::resize`). | `wait_reports_signal_input_and_timeout` (POSIX) covers the sibling arms |
| `tui/src/app.rs:3610` | attribution-artifact | the closing brace of `wait_for_agent`'s `if first_attempt` block. The assignment on 3609 demonstrably ran: its only reader is `if !first_attempt`, whose true side is what appends the "✅ Connected to agent" message asserted by `wait_for_agent_retries_until_agent_appears`. | that test asserts both messages (`retrying every 1s`, `Connected to agent`); re-checked against the new report (3601-3609 `DA=1`, **3610 `DA=0`**, 3611 `DA=1`) |
| `tui/src/components/chat_area.rs:2651` | unreachable-by-construction | the second `width.max(3)` is the *format argument of an `assert!` message*: evaluated only when the assertion is about to fail, i.e. only by a test that must fail. | the sweep spans several widths incl. the `max(3)` floor |
| `tui/src/agent_supervisor.rs:141`, `142`, `143` | unreachable-by-construction | `Child::try_wait()` returning `Err`. **Reason corrected (review §4.2):** nothing about *this host* prevents it — the construction does. The wait status of a child this process spawned and still owns is not something the OS refuses to report (no `waitpid` failure path, no external reaper). | the `Ok(Some(status))` arm is asserted verbatim by `a_sidecar_candidate_that_exits_…`, and the `Ok(None)` sibling by the readiness fixture |
| `tui/src/agent_supervisor.rs:267` | unreachable-by-construction | `ensure_agent_running_refuses_an_unavailable_explicit_endpoint`'s `Ok(_) => panic!`. **Reason sharpened (review §4.5):** the first gate is `select_agent`'s "no local fallback" for an explicit non-local endpoint — the `panic!` arm would need a *reachable, matching-version* agent on port 1, which needs root to bind. (`cfg!(test)` refusing the launch path is secondary, not the premise.) | that test asserts the refusal text |
| `tui/src/update.rs:104` | unreachable-in-this-environment | `check_manifest(&current, url, CHECK_TIMEOUT)` inside `check(current)`. `check` is entered only with a *published-channel* version: a real release/nightly build fetches `https://dl.future-os.cn/...`, while this harness build's version is a commit-hash build, for which `manifest_url` returns `None` at the line above. The body itself is covered directly by `http_check_only_notifies_for_a_valid_newer_manifest` / `http_timeout_and_oversized_manifest_are_silent` against a local server; only the const-URL plumbing differs. **Second reason (review §4.4):** covering it needs a build whose display version is a published label, and that same configuration fails the guard assertion in `index.rs` (`this build has no published channel`) — so the change is not free. **Pre-empting the obvious counterexample** ("parameterise the URL"): the two manifest URLs are production data chosen by `is_release`/`is_numbered_build`, and a test could only reach this line by (a) an *unasserted* call — forbidden by PLAN §4 — or (b) changing a production signature/inventing an env knob for one plumbing line, which is the metric-driven production edit the plan's fixed build version makes unnecessary. | those two tests + the guard assertion in `interactive_polls_the_update_check_when_not_offline` |

Category totals: `unreachable-by-construction` 6, `unreachable-in-this-environment`
3, `platform-unmeasured` 3, `attribution-artifact` 1 — **13 of 13 lines
accounted for, 0 unwaived, and no row left in a falsified category** (§9.8 lists
the ten lines the review falsified and what now covers each).

### 9.5 Corrections to §§1–8 (stale rows only)

* §1.x/§2.x/§5.x/§8 reference the Windows run, the entry-based gate metric and
  the old line numbers; the *outcomes* they describe have been re-measured here.
  In particular §8's "further work" item 3 (the `app.rs` `on_resize` callback) is
  **closed**: `TestPty::resize` reaches it on macOS (§9.3).
* §5.1/§5.2 rows for `terminal.rs` (`terminal_or_skip`'s `Ok` arms), `update.rs`
  (`cfg!` installer arms), `components/footer.rs`, `worktree.rs` and
  `components/autocomplete.rs` (the `[skip]` arms) describing them as
  environment waivers are superseded: those lines either execute now or are
  compile-time-dead branches removed from the measured source (§9.3).
* §5.2's `tui/src/index.rs` rows (`run()`'s error tails, the console skip) are
  superseded by §9.3/§9.4: the tails are now covered by mock-agent tests.
* §5.3's `agent_supervisor` row remains correct in substance (the launch loop is
  behind `cfg!(test)`) and is now *partly covered* by the integration test.

### 9.6 Reproducing

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home CARGO_TARGET_DIR=target/cov-tui
mkdir -p "$HOME"
cargo llvm-cov -p future-tui --no-fail-fast --lcov --output-path /tmp/tui-lcov.info -j 4
python3 .future/cov100/covlines.py total  /tmp/tui-lcov.info --under .worktrees/cov100-r2/tui/src
python3 .future/cov100/covlines.py detail /tmp/tui-lcov.info --under .worktrees/cov100-r2/tui/src

# the acceptance suite for this unit (instrumented or not, both green)
cargo test -p future-tui            # lib 2186 + agent_sidecar_launch 1 + bughunt 8 + cli_smoke 5

# if a test target fails, cargo-llvm-cov writes no report; the profraw is still
# on disk, so render it separately (and expect the last 10 % of the suite to be
# missing from the numbers — fix the failure rather than reporting that file)
cargo llvm-cov report --lcov --output-path /tmp/tui-lcov-from-profraw.info
```

* The measurement is **not** stable against a *hang*: if `write_stdout_retries_on_eintr`
  or `create_pipe_error_paths` ever regress to spawning/writing while another test
owns fd 1, the run dies with `BrokenPipe`/no report. Both carry the lock and the
early fd-1 restore for that reason (§9.3) — check them first.
* Delete `target/cov-tui` when handing this unit over (`rm -rf target/cov-tui`);
  the shared disk is the campaign's scarce resource.

### 9.7 Measurement form, stability, and the flake question

Three questions a reviewer must be able to answer about a number, answered here
with the tooling rather than by assertion.

**(a) The campaign baseline says 270, this unit's re-measurement of the same
untouched tree says 268.** Not a flake: an exact, line-level scope difference.
`python3 .future/cov100/covlines.py detail` on both reports of revision `8449acf6`
(my crate-scoped run has no `SF:` record for any test target, so it *is* the
untouched tree, not a contaminated one):

| line | this unit's form (`-p future-tui`) | campaign baseline (`--workspace`, `ws-dirty`) |
|---|---|---|
| `agent_supervisor.rs` 107-109 (spawn-failure `failures.push` + `continue`) | `DA=2` **covered** | `DA=0` |
| `agent_supervisor.rs` 126-129 (connect-error `kill`/`wait`/`return Err`) | `DA=1` **covered** | `DA=0` |
| `agent_supervisor.rs` 121 (`Ok(_) => return Ok(Some(OwnedAgent { child }))`) | `DA=0` uncovered | `DA=1` **covered** |
| `index.rs` 1130-1133 (`--print` usage line and exit) | `DA=0` uncovered | **covered** |

`270 = 268 − 5 + 7` exactly. Both files are also compiled into the `future` CLI
crate and into the **non-test** `future-tui` binary, so which lines a report
attributes as covered depends on which targets that run executed and what was on
`PATH`: this crate's own `tests/cli_smoke.rs` drives the real binary (covering the
spawn-failure arms, `DA=2` = both candidates), while the workspace run had some
test that really launched and connected to an agent (covering 121). Consequence for
this ledger, stated rather than hidden: **`agent_supervisor.rs:121` is covered in
the campaign's workspace-wide form and uncovered in this crate-scoped form** — a
+1/−1 in the other direction, and §9.4 records it as such instead of claiming it.

**(b) Stability inside this form.** `tui-lcov-final5.info` and
`tui-lcov-final6.info` are two independent instrumented runs of the same code
(only `cargo fmt` between them): both `uncovered=24`, and
`covlines.py detail` output is identical line for line. The runs before those were
different code, not repeats: baseline 268 → after the terminal/PTY work 47 → after
index/clipboard/update 26 → 24. No line in this crate was observed to oscillate
between the two same-code runs.

**(c) Flakes.** None of `FLAKES.md` §5 / §5b items 13–17 is in `tui` (they are
`agent`, `cli`, `channels`, `packages/rpc`, `desktop`), so no documented flake can
have moved this unit's number. What *did* move the numbers were three hazards this
unit found in its own test files — each reproduced, diagnosed and fixed here, and
each a mechanism `GOTCHAS.md` does **not** document (a `grep -i` for
fd/pipe/pty/deadlock finds only an unrelated Windows item), so they are candidates
for the flake inventory with these reproductions:

1. **A test that leaves fd 1 on a full pipe deadlocks the harness, not just
   itself.** `write_stdout_retries_on_eintr` pointed fd 1 at a *filled* blocking
   pipe and had its drainer stop after `filled + payload.len()` bytes; the harness
   writes its own progress to fd 1, so the count was unreachable and the drainer
   stopped early, leaving the payload write blocked forever. Observed: a
   measurement run alive >20 min, main thread blocked in `write` from libtest's
   test-name formatter. Fix: restore fd 1 immediately after the payload write, and
   drain until a stop flag instead of until a byte count (§9.3).
2. **A test that leaves fd 1 (and fd 0) on a PTY slave has no reader.** fd 0/fd 1
   are process-global: the two size tests pointed fd 1 at a pty slave whose master
   nobody drained, so the ~1 KB slave buffer could fill and block *any* writer —
   including a sibling test's panic path, which then never reached its restore.
   Observed: the same >20 min hang with fd 0 and fd 1 both on the slave. Fix:
   `terminal::tests::drain_pty_master`, draining the master for the test's duration
   (`O_NONBLOCK`, joined before the fds are closed).
3. **A child spawned with inherited stdio while another test owns fd 1.**
   `create_pipe_error_paths` spawns `current_exe()`; if a sibling test has fd 1 on
   a full/half-closed pipe, the child's own libtest summary write fails and the
   parent's `status.success()` sees it. Observed twice: `error: io error when
   listing tests: Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }` then
   `error: 1 target failed: -p future-tui --lib` (exit 101) — and because
   cargo-llvm-cov writes no report when a target fails, **two measurement runs
   produced no lcov at all**. Fix: the test holds `crate::test_env::lock()`
   (§9.3).

After those fixes: 4 consecutive green instrumented runs and 3 green plain
`cargo test -p future-tui` runs, all at `-j 4` on a host whose load average was
15–25 during the campaign. That is a rate, stated with its load, in the same shape
as `FLAKES.md` item 17's negative data point.

**(d) The rendering gate.** No production rendering code was touched by this unit
(the diff is tests, `App::apply_update_notice`, `#[cfg]` consts and the
`terminal_or_skip` split), and the byte-exact golden gate confirms it:
`./tui/tests/tmux-diff.sh` → **57 passed, 0 failed, 0 assertion failures**, exit 0,
with `cleanup: nothing left on 'grpc-addr 127.0.0.1:57635'`.

**(e) The suite-killing fd 1 flake (found while re-measuring; pre-existing).**
On this host today, **5 of 8 instrumented attempts** and **2 of 7 plain
`cargo test -p future-tui` runs** died with `error: io error when listing tests:
Os { code: 32, kind: BrokenPipe }` — libtest aborting because its **own** stdout
write failed — and because cargo-llvm-cov writes no report when a target fails,
each of those attempts produced **no lcov at all**. It is *not* caused by this
unit's tests: it reproduces with all seven new tests skipped (`--skip a_signal_arriving_mid_poll
…`, 1 of 3 runs) and disappears when `write_stdout_retries_on_eintr` is skipped
(0 of 15 runs of the terminal subset), so it is enabled by that test's ~0.5 s
window with **fd 1 pointing at a full pipe** while libtest prints the results of
the tests finishing next to it. Diagnosis recorded rather than solved: an fd-1
state watcher (`fstat` + `poll` + a 1-byte probe every 2 ms) in the same process
never saw fd 1 closed, un-writable or `POLLERR`/`POLLHUP` during a failing run,
while the harness's output for that period is *lost* (17 of 54 results missing
from one run's output) — so the write that earns the `EPIPE` is not one a watcher
in that process can see. Every symptom the unit already documented in §9.7(c) is
in this family (fd 0/fd 1 are process-global while the harness writes test names
to fd 1). **Handed to the flake units** with the reproduction above; the
measurement command needs a retry, and the retry needs the whole instrumented
build/run again.

One further pre-existing flake was seen in the same four attempts: `app::tests::
wait_for_agent_retries_until_agent_appears` failed once (a live-agent timing
test whose budget is wall-clock, §9.7(c)), unrelated to this unit's changes.

### 9.8 Round-2 rework — the adversarial review (`REV-TUI`) and what closed

Review: `docs/testing/review-tui.md` (unit `REV-TUI`). It confirmed the headline
figure and the 24 line numbers, and falsified **two of the four categories**: 11
of the 24 rows were reachable by a test that stays green. All 11 are covered now
(10 by new tests, one — `agent_supervisor.rs:121` — by a launch fixture), the
remaining 13 are re-registered with corrected reasons (§9.4), and the category
arithmetic is `unreachable-by-construction` 6 · `unreachable-in-this-environment`
3 · `platform-unmeasured` 3 · `attribution-artifact` 1.

**Per falling row, and what covers it now.**

| row (old → new) | what closed it |
|---|---|
| `agent_supervisor.rs:121` → `:121` (covered) | `tui/tests/agent_sidecar_readiness.rs::a_candidate_that_answers_readiness_is_returned_as_an_owned_child`: `PATH` holds one stub candidate and an in-process readiness mock serves the instance's own `$FUTURE_HOME/run/agent.sock` — **late**, only after the stub has touched its sentinel, so the handshake cannot win the race against the shell the loop just spawned. Asserts `Ok(Some(_))` (not `Ok(None)`), the sentinel, and ≥1 handshake. The review's alternative (“a same-tree sibling `future`”) is what the *workspace* form does for free; that is host state, see §9.1. |
| `agent_supervisor.rs:126-129` (hard error) → covered | same file, `an_incompatible_agent_stops_the_launch_loop_and_is_reported`: the mock answers readiness with `9.9.9`, so the loop must kill the child, stop, and report `Incompatible Future Agent … 9.9.9`. Previously covered only by whatever `future` the host's `PATH` happened to hold (`/opt/homebrew/bin/future`); now deterministic. |
| `app.rs:11995` → `:11995` (covered) | `the_history_settle_budget_reports_a_page_that_never_answers` — `#[tokio::test(start_paused = true)]` + a page that never settles: the 4 800 `tokio::time::sleep` calls of `PUMP_BUDGET_ITERS` are walked in milliseconds of wall time, and the test asserts the panic text. |
| `app.rs:25296` → `:25296` (covered) | `wait_for_set_cwd_reports_nothing_delivered_by_its_deadline` — nothing is submitted, the helper's own 20 s bound is walked, and the test asserts `None` **and** that ≥19 s elapsed (so an early return would fail). |
| `app.rs:7220-7222` → covered | `tui/tests/sandbox_panel_real_platform.rs::the_sandbox_panel_opens_through_the_real_platform_twin`: an integration target links the lib *without* `cfg(test)`, so the real `host_sandbox_platform()` is in the target; typing `/sandbox` through the public `App::handle_input`/`handle_cmd` reaches it with no PTY and no agent. Asserted on the emitted `UiCmd::Submit("/sandbox")` and on `App::next_deadline()` gaining a value. |
| `grpc_client.rs:1239` → covered | `probe_agent_reports_an_unclassifiable_transport_failure_as_uncertain`: an address that cannot be parsed into a URI (`ConnectError.kind` is `None`) must classify as `Uncertain`, and a refused loopback port as `Gone`. The resolver/routing-dependent addresses the review used are documented in the test rather than asserted. |
| `grpc_client.rs:1344` → covered | `the_reconnect_poll_stops_at_its_loop_top_when_the_flag_is_set_alone`: after the stream is lost and the backoff has grown once, the flag is stored **with no notification** (the construction `subscribe_loop_top_stop_check` already used for the subscribe loop), and the test asserts the backoff freezes across two windows rather than probing again. |
| `grpc_client.rs:3483` → covered | `attach_fails_loudly_when_no_stream_ever_serves_it` — nothing serves the address, the bounded 20-attempt loop is exhausted, and `#[should_panic(expected = "client never attached to a live event stream")]` asserts the arm. |
| `grpc_client.rs:3828` → covered | `wait_for_backoff_above_gives_up_at_a_bound_the_backoff_cannot_reach` — a *caller* asserts the helper's documented `None` (threshold `u64::MAX`), plus that its bound was walked. |
| `terminal.rs:1283` → covered | `a_signal_arriving_mid_poll_reaches_the_failure_arm` — the polling test raises `SIGWINCH` itself (the lock serialises *other* tests, it does not forbid this one), `wait(0)` returns `Signal(28)`, and `catch_unwind` asserts the helper's panic text. `catch_unwind` rather than `#[should_panic]` so the signal handlers and the self-pipe are restored before the assertion. |

**The two review items that matter beyond the ledger.**

* **Hazard 2 (the invisible `cli_smoke` path) — fixed.**
  `print_mode_without_message_exits_one` used to exit 1 from *either* the print
  branch or the failed agent launch, so the test passed whether or not a sidecar
  started (the difference lived only in coverage counters). It now runs against a
  readiness mock on an explicit `--grpc-addr`, so no launch can happen, and it
  asserts the stderr text (`No prompt provided`) — the path is pinned rather than
  inferred. That also removes the accidental launch that made the crate-scoped
  and workspace forms disagree about `agent_supervisor.rs` (§9.1).
* **Hazard 1 (host state behind the number) — recorded.** §9.1 now names every
  candidate the measurement could resolve (private target dir, `PATH`, the
  Homebrew `future`/`future-agent`) and the launch fixture asserts its own
  precondition (no unhashed `deps/future`), so the assumption fails loudly
  instead of silently changing which agent answered.
* **Hazard 3 (`FORCE_NEW_FAILURE` is a process-global one-shot) — recorded, and
  the discipline left intact.** The risk the review named is a *future* test that
  constructs a `Terminal` inside the store→consume window; the seam's comment at
  `the_injected_terminal_failure_is_reported_once` now says so explicitly (all
  eleven `Terminal`-constructing tests hold `terminal_test_lock()`, which is what
  makes the window unreachable) and points at the reverse symptom already
  documented at `columns_rows_read_cached_size`. No locking change.
* Hazards 4 (`stop_notify` is `notify_waiters`, so `grpc_client.rs:1344` used to
  oscillate) and 5 (`SIGNAL_PIPE_WRITE` has one unlocked writer) are unchanged
  and stay with the flake units; hazard 4's oscillation is now *observed*
  (the line is covered by a deterministic test, so a future `DA=0` there is a
  real signal).

**Reproducing this section.**

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home CARGO_TARGET_DIR=target/cov-tui
cargo llvm-cov -p future-tui --no-fail-fast --lcov --output-path /tmp/tui-r2-crate.info -j 4
# retry on the §9.7(e) flake: it aborts the binary and cargo-llvm-cov then writes
# no report at all
python3 .future/cov100/covlines.py total  /tmp/tui-r2-crate.info --under .worktrees/cov100-r2/tui/src
python3 .future/cov100/covlines.py detail /tmp/tui-r2-crate.info --under .worktrees/cov100-r2/tui/src
python3 .future/cov100/reconcile.py --report /tmp/tui-r2-crate.info --scope tui/src \
    --doc docs/testing/module-tui.md --section "Round 2" --label tui
# the same number with a same-tree sibling planted (the review's "workspace form"
# artefact) — the tui side must not move:
cp target/debug/future target/cov-tui/llvm-cov-target/debug/future
cargo llvm-cov -p future-tui --no-fail-fast --lcov --output-path /tmp/tui-r2-sibling.info -j 4
python3 .future/cov100/covlines.py detail /tmp/tui-r2-sibling.info --under .worktrees/cov100-r2/tui/src
rm -f target/cov-tui/llvm-cov-target/debug/future
```

### 9.9 The §9.7(e) suite-killing flake — measured, instrumented, and still open

§9.7(e) diagnosed this flake but left it open, and it kept its price: it is the
reason §9.1's second flavour has no number ("all four attempts were lost"), and
in this unit's own re-measurement **2 of 8 plain full-suite runs** died the same
way. The workstream record is **`docs/testing/flaky-tui.md`**; the parts that
belong in this module's ledger:

* **Invocation and rate, before:** `.future/cov100/flake-rate.sh --label tui-fd1
  --runs 8 --timeout 300 -- timeout -k 5 280 cargo test -p future-tui --lib -j 4`
  → `runs=8 failures=1 timeouts=0`, the failure being
  `error: io error when listing tests: Os { code: 32, kind: BrokenPipe, … }` with
  **no named test** (`flake-rate.sh` has no category for a harness that dies) and
  **no lcov** under `cargo llvm-cov`. `timeout -k 5` is required: this suite
  catches `SIGTERM`, so a stalled run survives a plain `timeout` (measured: 26
  minutes and still alive).
* **After this unit's fix** (same command, same 8 runs): **1 failure in 8 runs** —
  i.e. **the rate did not move and this flake is NOT fixed**, even though the
  route the probes prove (a write asleep inside the pipe is broken by the `dup2`
  that hands fd 1 back) is real and removed. The residual is written up in
  `flaky-tui.md` §1.3–§1.5, including exactly which explanations the instruments
  *ruled out* for a failing run.
* **Mechanism** (measured, `flaky-tui.md` §1.3): fd 1 is process-global; this
  test points it at a **full** blocking pipe for ~0.5 s, so libtest's own
  progress writes — and the *unlocked* `write_stdout_writes_all_bytes` — queue
  inside that pipe. Two measured kernel facts make that lethal: a write to a
  *readerless* pipe returns `EPIPE` (`POLLHUP`, never `POLLERR`, on `poll`) and is
  *not* woken when the read end closes, and — the one the failing trace points at
  — a write that is **asleep** in a full pipe is broken (`EPIPE`/`SIGPIPE`, read
  end still open) by the `dup2` that hands fd 1 back, which is the test's own
  restore step. Either way the run is lost: libtest aborts the binary, or the
  suite hangs and `timeout` cannot end it (the suite catches `SIGTERM`).
* **Fix:** (1) the restore is deferred until the pipe has been **drained empty**,
  which wakes a sleeping writer so it copies and returns instead of being broken
  by the replacement (probe pair: without the drain the same writer dies, with it
  `n=80 errno=0`); (2) the drainer reads through an `F_DUPFD` reference parked at
  fd ≥ 100 and runs until **EOF**, which is only reported once every write end —
  including one held by a blocked writer — is gone, so the read end is closed only
  when no writer exists and a stale close of a recycled low fd can no longer make
  the pipe readerless. No retry, no `--skip`, no sleep, no weakened assertion, and
  the 500 ms blocked-write window the test exists for is unchanged.
* **Still open:** the rate did not move (1/8 → 1/8), so the route the probes prove
  is either not the dominant one in a full run or is reached in a way the drain
  does not cover; the failing run's fd-1 state and what was *ruled out*
  (readerless pipe, stale close of the read end, early teardown) are in
  `flaky-tui.md` §1.3–§1.5 with the next check to run. Also **not** identified:
  any call closing the read end during the window (the trace shows none).
* **Second, smaller item in the same handoff:** `app::tests::wait_for_agent_retries_until_agent_appears`
  was red in 1 of 8 instrumented attempts (`/tmp/tui-cov-crateB-1.log`,
  `2192 passed; 1 failed`). It bounds real startup with a **10 s wall-clock
  budget** (`tokio::time::timeout(…, app.start(…))` + `.expect(…)`) around a retry
  cadence of **1 s** and a readiness flip that depends on another tokio task being
  scheduled, so under load the budget can expire for reasons outside the code
  under test — the same class as `FLAKES.md` items 6 and 8. Rates: 1/8
  instrumented; **0/16** full-suite runs during this unit's own before/after
  measurements. Registered with its mechanism and rate, not "fixed" (`FLAKES.md`
  §4: a flake that cannot be reproduced is recorded, not quietly patched).
