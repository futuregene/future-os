# future-cli + future-tui — coverage, waivers and test-quality record

Goal `cov-100-multidim` · modules **future-cli** (`cli/`) and **future-tui** (`tui/`) ·
branch `test/cov100` (worktree `.worktrees/cov100`) · host **Windows x86_64**, rustc
1.97.0 pinned by `rust-toolchain.toml` · measured 2026-09-26 · session
`20260926-013544-d1be54d36ab240b3b1e41144314a3c33`.

Evidence literals for this handoff: **windows-tests-green** (§3), **dimensions** (§4),
**weak-tests-fixed** (§5), **lines-100-or-waived** — **not yet satisfied**, see §6.

## 1. Measurement (exact commands)

`cargo llvm-cov` 0.9.1 has **no `--target-dir` flag** (the plan's form is rejected:
`error: invalid option '--target-dir'`); the private target dir is `CARGO_TARGET_DIR`,
and the tool appends its own `llvm-cov-target` subdirectory to it:

```powershell
$env:CARGO_TARGET_DIR = "target/cov-tui"      # private: never the shared target/
cargo llvm-cov -p future-cli -p future-tui --json --output-path coverage/tui-report.json
python .future/cov100/verify.py crate future-tui 99.99 coverage/tui-report.json
python .future/cov100/verify.py uncovered future-tui coverage/tui-report.json   # and future-cli
python .future/cov100/verify.py windows-red-baseline future-tui                  # §3
```

Report: `coverage/tui-report.json` (private; one run covers **both** crates of this
task's scope, which is what `verify.py crate` requires — a tui-only report is rejected
with *"does not cover future-cli"*). Never `coverage/llvm-cov-full.json`.

Gotcha seen twice: with the same `CARGO_TARGET_DIR`, a `cargo llvm-cov -p future-tui`
run *cleans* the profile dir, and a following `cargo llvm-cov -p future-cli --no-clean`
still produced a report with **only** the cli files. `--no-clean` cannot be combined
with `--no-report`. Always re-run both packages in **one** invocation and report from
that run.

## 2. Result — measured, not estimated

| module | covered / total lines | line % | uncovered | files with gaps |
|---|---|---|---|---|
| future-cli | 22958 / 23923 | 95.9662 % | 965 | 26 of 55 measured |
| future-tui | 58292 / 59379 | 98.1694 % | 1087 | 31 of 49 measured |

There is no usable *pre-segment* pair for these two crates: the supervisor's
whole-workspace report (`coverage/llvm-cov-full.json`, 2026-09-26 00:51) has the crate
binaries **built but never executed** — future-tui 0 / 59292 lines, future-cli
170 / 24489 — because an earlier failing target stopped that run before the tui/cli
test targets executed. That is exactly why the Windows red line (§3) had to be fixed
before any measurement of these two crates could mean anything. My first measurement
after the fix (before the two weak-test rewrites of §5) was 58284 / 59373 =
98.1658 % for tui and 22946 / 23908 = 95.9762 % for cli; the delta of this segment is
therefore *the whole measured baseline*, not an improvement on a previous number.

Suite state on this host after the changes (§3 + §5), from the same run:

| target | tests | result |
|---|---|---|
| `future-cli` lib | 656 | ok |
| `future-cli` main / `bin.rs` / `event_wire_contract.rs` | 2 / 11 / 2 | ok |
| `future-tui` lib | 2035 | ok |
| `future-tui` `bughunt_regressions.rs` / `cli_smoke.rs` | 8 / 5 | ok |

## 3. The Windows red line — 13 tui failures + 1 cli failure, now green

`python .future/cov100/verify.py windows-red-baseline future-tui` →
**13 green, 0 still red, 0 not runnable**. Every fix below keeps the original assertion
(§3.9 lists the one place where a *racy* assertion was replaced by a stronger,
deterministic one); no guard was deleted, no `cfg(test)` was added around production
code, no assertion was loosened.

### 3.1 Clipboard basename vs absolute path — 2 tests

`app::tests::ctrl_v_attaches_the_image_on_the_clipboard` and
`paste::tests::a_png_on_the_clipboard_becomes_an_attachment_path` compared the
attachment *name* against `path.rsplit('/').next()`; on Windows the path is
`C:\…\clipboard-….png`, `rsplit('/')` finds no `/`, and the whole path came back. Fixed
by taking the platform's own file name
(`Path::new(&path).file_name()`), which asserts the same claim — *the label is the
file's name* — and is now host-independent.

### 3.2 `paste::tests::path_candidates_cover_the_five_paste_shapes`

The fixture hard-coded POSIX `/tmp/a.png`, which is **not** `Path::is_absolute()` on
Windows (`path_candidates` correctly returns `["C:/…"]` only for drive-rooted paths).
The five shapes are now built from a host root (`C:/tmp` on Windows, `/tmp` elsewhere)
and a host `file://` prefix (the extra slash before a drive letter is the URL's). Same
cases, same expectations, both hosts.

### 3.3 `app::tests::editor_command_refills_the_draft_from_the_temp_file`

`/editor` really spawns `$EDITOR` (only `edit_draft_with`'s launch is injectable) and
the fixture used `EDITOR=true` — Windows has no `true`, so the spawn failed with
`program not found` and `Draft updated` never appeared. New helper
`noop_editor()` returns `cmd /c rem` on Windows and `true` elsewhere; it is a no-op
that exits 0 on each host. The three editor tests use it.

### 3.4 Dead-client error paths were read 375 ms too early — 5 tests

`usage_command_opens_the_usage_panel`, `providers_list_reports_a_failed_load`,
`sandbox_panel_warns_about_the_unknown_tier_and_shows_a_downgrade`,
`permission_selected_in_the_panel_applies_and_mirrors` and `slash_commands_matrix` assert
that an RPC to the unreachable client produces its failure message. They used
`pump()` (a 375 ms quiesce) and then read the transcript; on Windows a **refused
loopback connect takes ≈2.2 s** (measured: `TcpClient.ConnectAsync("127.0.0.1",1)`
faulted after 2189 ms), so no message had arrived. Each now uses the file's existing
bounded waiter `pump_until_msg(…, "<the message>")` (120 s budget), which asserts the
same condition instead of guessing the timing. This is the same pattern the file already
used for `/export`'s failure elsewhere.

### 3.5 Two failure messages raced a second in-flight RPC

In the sandbox-panel test the earlier policy write and the later probe both fail
against the dead client, and llvm-cov's slower schedule can land them in either order;
`last_system(&app)` then read the *other* failure. Both that test and the permission
test now assert `system_messages(&app).iter().any(…)` — the claim under test ("the
failure is reported") — instead of an ordering that neither RPC controls. (The plain
run passed; the instrumented run is where the race showed. Both are green now.)

### 3.6 `git worktree list` prints a different spelling than the fixture — 2 tests

`fs::canonicalize` adds the `\\?\` verbatim prefix on Windows, which git never prints,
so `list[0].path` (`C:/Users/…`) never equaled the fixture's `\\?\C:\Users\…`, and
`Path::starts_with` treats the two prefixes as different components. The fixtures
(`worktree::tests::TempRepo`, `app::tests::TempGitRepo`) now canonicalise **only on
non-Windows** (the macOS `/var` → `/private/var` reason is the only reason to resolve
at all), and the path comparisons go through a new test helper `location()` that
normalises separators and a leading `//?/` before comparing — `Path` equality is
component-wise, so `C:/a/b` and `C:\a\b` compare equal, while a genuinely different
directory still fails.

### 3.7 `app::tests::creating_a_worktree_runs_git_then_switches_the_session`

With the fake git its planned path is `Path::new("/repo").join(".worktrees").join("demo")`,
whose `display()` is host-spelled (`/repo\.worktrees\demo` on Windows) while the test
waited for the literal `/repo/.worktrees/demo`. New bounded waiter
`pump_until_workdir(app, rx, expected_path)` waits for a `Working directory:` line whose
path is the *expected location* (via `location()`); the recorded `set_cwd` command is
compared the same way. The exact `git` argv assertion is unchanged.

### 3.8 `app::tests::a_real_repository_creates_a_worktree_and_switches_to_it`

Same cause: the app echoes the path it planned from **git's** spelling while the test
built its needle from the canonicalised fixture root. Now uses
`pump_until_workdir` + `location()` for both `app.state.cwd` and the `set_cwd` payload.
The real `git rev-parse --abbrev-ref HEAD == "feat/demo"` assertion is unchanged.

### 3.9 Where an assertion was replaced, and why that is stronger

The two `last_system` assertions of §3.5. `last_system` additionally required the
failure to be the *last* message — a property no test can control while a second RPC is
in flight, and one that would fail a correct implementation. `any(…)` asserts the
report exists; the failure output prints the whole transcript, so a regression still
shows exactly what was reported instead.

### 3.10 `cli::commands::doctor::tests::doctor_all_checks_passed` (cli)

Not in the 13-entry baseline, but it was the **only** `future-cli` failure on this host
(655 passed / 1 failed), and the acceptance contract requires both crates green. The
fixture writes five extension-less fake binaries and prepends their directory to `PATH`;
`cli::utils::files::which` runs **`where <name>.exe`** on Windows, so the fixtures were
never found and the transcript said `future-channel not found on PATH`. The fixture now
appends the host's executable suffix (`format!("{name}{suffix}")`, `.exe` on Windows).
`which`'s own tests are untouched.

## 4. Dimensions (matrix rows for future-cli and future-tui)

| module | dimension | evidence (tests that make the claim real) |
|---|---|---|
| future-cli | boundary | `commands::session_history::tests` cursor/limit edges; `utils::string::tests` (empty/one/very long); `browser::selector::tests` selector grammar; `commands::doctor::tests::doctor_all_checks_passed` (0 providers, 0 sessions); `commands::skills::tests` excerpt cap (`parse_catalogue_caps_the_excerpt_in_its_error`) |
| future-cli | error-path | `commands::doctor::tests::doctor_clean_environment_output` (nothing installed), `doctor_config_issue_variants`, `doctor_invalid_auth_json_is_tolerated_by_providers_check`, `doctor_captures_partial_version_from_hanging_binary`; `commands::session::tests::set_json_reports_applied_and_failed_fields`; `utils::files::tests::assert_readable_file_errors`; `browser::discovery::tests::first_existing_scans_in_order_and_may_miss` |
| future-cli | concurrency | `browser::browser_state::tests` (config lock is held across the transaction); `commands::run` deadline/cancel arms (`GRPC_DEADLINE_SEC`); `test_server` mock agent serves concurrent calls in the doctor/provider tests |
| future-cli | property | `utils::string::tests` table-driven over input classes; `browser::selector::tests` round-trips selector text; `commands::models::tests` provider/model JSON round-trip; `doctor` provider counting is asserted against a fixture, not a snapshot |
| future-cli | platform-cfg | `utils::files::tests::which_finds_shell` (host shell), `browser::windows_process::tests`, `browser::safari::*` (macOS WebDriver paths, by construction), `commands::config` path resolution; **this segment**: `browser::discovery::tests::discovery_runs_platform_candidates` and `browser::chromium::chromium_manager::tests::launcher_lookup_platform_discovery_runs` now assert what a host-dependent discovery result *must* satisfy instead of merely calling it |
| future-cli | serialization | `commands::models` / `commands::session` JSON shapes; `test_server::HttpRoute::json` fixtures decode real agent payloads; `event_wire_contract.rs` (2 tests) pins the stream-event wire shape; `doctor` parses `auth.json`/`models.json`/`settings.json` and reports malformed ones |
| future-tui | boundary | `components::diff::tests::rendered_rows_have_exact_width_for_apply_patch_and_wide_glyphs` (widths 1…60, CJK); `components::markdown::tests` table width shrink/CJK wrap; `insert_history` scroll edge; `paste::tests::path_candidates_*` (empty, one, max, Unicode, quoted, `file://`); `components::chat_area::tests::tiny_terminal_width_does_not_underflow` |
| future-tui | error-path | `app::tests` dead-client failures (`Failed to export session`, `Failed to load usage`, `Sandbox probe failed`, `Failed to set the permission level`, `Failed to load providers`), `editor_flow_reports_a_spawn_failure_and_cleans_up`, `rpc::grpc_client::tests::transport_error_detection_matches_ts`; `terminal::tests` injected `FORCE_NEW_FAILURE`; `worktree::tests::a_failed_query_returns_the_git_error_not_a_partial_answer` |
| future-tui | concurrency | `app::tests` streaming/`esc`/queueing tests; `terminal::tests` burst-window (`InputSink`) and the panic-restore `try_lock` path; `agent_supervisor` startup/dedup; `skills_cli`/`skill_reco` prefetch cancellation (`a_failed_prefetch_is_silent_and_not_retried`) |
| future-tui | property | `components::chat_area::tests::expect_streaming_matches_full_render` — incremental streaming output must equal the eager render (assertion *helper*, see §5); `paste::tests::sync_image_markers` invariants; `worktree::parse_worktree_list` porcelain round-trip; `markdown`/`diff` width invariants over a table of cases |
| future-tui | platform-cfg | `terminal_or_skip()`/`backend_or_skip()` gate console-dependent tests (proved: `[skip] terminal test needs a console: stdin/stdout are not console handles — the interactive TUI needs a terminal`); `terminal_posix.rs` is `#[cfg(unix)]` and absent from this report; `parse_worktree_list`/`normalize_path` have `cfg(unix)`/`cfg(windows)`-specific fixtures; **this segment**: `paste::tests::path_candidates_cover_the_five_paste_shapes` and the worktree path comparisons are host-spelled instead of POSIX-hard-coded |
| future-tui | serialization | `rpc::grpc_client::tests` typed-first decoding with a JSON `data` fallback; `rpc::provider_types::tests::validate_accepts_a_complete_provider`; `skills_cli::tests::parse_catalogue_*` (alias fields, unknown fields, nulls); `app.rs` tolerant parsers (`get_state` shape errors); `event_wire_contract`-style frame assertions in `bug_hunt_regressions.rs` |

## 5. Weak-test audit — weak-tests-fixed

Method: the detector `verify.py weak` uses (`#[test]`/`#[tokio::test]` bodies without an
assertion token), scoped to `cli/` + `tui/`. `docs/testing/weak-test-audit.md` is a
separate, shared deliverable and does not exist yet, so the scan is reproduced read-only
here. Census: **1991** test functions, **25** flagged.

Audited by hand, the detector's 25 split into three groups — the detector only matches a
narrow token list and counts braces naively, so it both misses real assertions and
truncates bodies that contain `{`/`}` inside string literals:

| group | n | detail |
|---|---|---|
| real assertion, missed by the detector | 10 | `#[should_panic(expected = …)]` (`clipboard::forbidden_runner_aborts_when_a_branch_reaches_it`, `keymap_view::the_highlight_walker_*`, `terminal_posix::panic_restore_raw_skips_when_lock_held`), `.expect(…)`/`.unwrap()` (`clipboard::native_runner_round_trips_through_a_real_program`, `rpc::provider_types::validate_accepts_a_complete_provider`, `terminal_posix::write_stdout_writes_all_bytes`), assertion *helpers* (`components::diff::rendered_rows_have_exact_width_…` → `assert_rows_width`; `components::chat_area::{nested_indented_fence_stream_matches_eager_render, link_reference_definitions_disable_prefix_caching_safely}` → `expect_streaming_matches_full_render`), and `skills_cli::parse_catalogue_rejects_output_without_a_json_object` whose `assert!(result.is_err())` sits *after* a `"}{"` fixture that makes the brace counter stop early |
| **fixed in this segment** | 2 | `browser::discovery::tests::discovery_runs_platform_candidates` — now asserts a discovery hit is a real file whose `infer_kind` matches its path (was `let _ = find_browser(None);`); `browser::chromium::chromium_manager::tests::launcher_lookup_platform_discovery_runs` — now asserts the three entry points agree on the same command, that it is a real file, and that `Launcher::discover` returns it with no args (was three `let _ = …;`). Both can fail: a wrong path/kind or a disagreeing wrapper fails the test. Each has a host-dependent arm (hit vs miss); on this host the *hit* arm runs and the miss arm's two lines are the 3 uncovered lines §6.2 records for `chromium_manager.rs` — that is a real branch, not a weakened assertion. |
| assertion-free, still open | 13 | `browser::chromium::cdp_event_router::dispatch_without_any_handlers_is_a_noop` (B — needs an observable from the router, e.g. a handler/registration counter); `components::autocomplete::{file_path_home_unset_falls_back_to_root, file_path_parentless_resolution_uses_dot}` (A — assert the returned items name paths under the resolved directory); `components::chat_area::tiny_terminal_width_does_not_underflow` (A — assert each rendered row's visible width); `components::markdown::parser_skip_arms_via_exotic_inputs` (A — the eight cases can assert their block structure, e.g. `"##\n"` yields no heading text); `components::scoped_models_selector::noop_callbacks_are_callable` (E — the closures are deliberate no-ops; the only assertion available is "callable without panic", which the test does make fail if the body panics); `crash::raw_write_stderr_writes_bytes` (C — needs fd 2 captured to assert the bytes); `terminal::noop_callbacks_invoke` (E — same no-op-callback shape as scoped_models_selector); `paste::tests` — see the 4 below; `terminal_posix::{signal_handlers_install_and_restore, restore_raw_handles_missing_pipe, signal_handler_ignores_unset_pipe}` and `terminal_posix::write_stdout_writes_all_bytes` (POSIX-only; **not compilable or runnable on this host**, so any rewrite would be unverified — handed to the POSIX-CI part of the work) |

This segment's larger test-quality contribution is §3: 14 previously red tests were made
deterministic *without* weakening them — two of them (`path_candidates_cover_the_five_paste_shapes`,
the two worktree path tests) now assert host-independent properties they did not assert
before, because the old form silently could not run where it mattered.

## 6. Waiver ledger and remaining-gap census

Total uncovered in this report: **2052 lines** — future-tui 1087 in 31 measured files,
future-cli 965 in 26 measured files. Gate status: **lines-100-or-waived is NOT
satisfied.** Three entries are waived (their lines are named below); the remaining 55
files are recorded as **open gaps** with no waiver claimed, because labelling testable
code as unreachable would be worse than an honest gap. `verify.py crate future-tui 99.99`
therefore fails *only* on the waiver gate, and the failure message lists exactly these
29 tui + 26 cli files (0 of them "not named in the doc").

### 6.1 Waived

| file | uncovered | category | lines | reason |
|---|---|---|---|---|
| `tui/src/terminal.rs` | 388 | `unreachable-in-this-environment` | 75-78, 127-129, 144, 162, 181, 183-185, 187-198, 207-209, 213, 218-219, 223-225, 229, 234-235, 240-243, 245-247, 250, 252-262, 264-267, 271-277, 281-283, 288-299, 301, 303, 305-308, 313-315, 317-322, 326, 328, 330-331, 339-343, 345-350, 352, 357-363, 367-371, 373, 375, 379-389, 392-397, 399-400, 404, 406, 408-409, 413, 415-417, 419-423, 427, 429, 431-438, 441-442, 444-445, 448-449, 451, 453-455, 458-459, 462-464, 466-470, 473, 476, 479-480, 482-484, 486-489, 493-499, 501-503, 505-507, 509-511, 513-518, 520-526, 528-530, 532-534, 536-538, 540-542, 544-546, 548-550, 552-570, 572, 574-581, 583, 585, 589-590, 592-593, 669, 761-764, 782, 788-793, 795-803, 959, 962-971, 974-976, 978-980, 982, 984-986, 997, 1000-1002, 1004-1007, 1059, 1063, 1065-1071, 1073-1074, 1362, 1365-1372, 1374-1378, 1391 | This is the real-terminal adapter (`Terminal::new/start/stop/write`, the key/ANSI writers, `restore_terminal_for_exit`) plus the console-gated *test* bodies that live in the same translation unit. It needs an attached console: on this redirected runner `Terminal::new()` fails and the module's own tests self-skip — reproduced with `cargo test -p future-tui --lib -- --nocapture terminal_default_simple_methods_and_progress` → `[skip] terminal test needs a console: stdin/stdout are not console handles — the interactive TUI needs a terminal` (test "passes" without executing its body). No injectable stdout/raw-mode seam exists; adding one is real design work, not a waiver. A console-attached Windows run or the POSIX CI (where `Backend::new` is infallible and `terminal_posix` is compiled) executes most of these lines. |
| `tui/src/terminal_windows.rs` | 167 | `unreachable-in-this-environment` | 98, 105, 110-117, 126-129, 136-144, 146-155, 157, 159-162, 167-168, 170-172, 177, 180-184, 191, 197-201, 208-211, 214-224, 226-234, 236-238, 246-249, 251-258, 260, 263-264, 266, 268, 270-278, 281-285, 289-291, 295, 300-314, 317-323, 325-326, 331-332 | The Windows console backend: `ConsoleHandle::new` (real `GetStdHandle`/`GetConsoleMode`), `read_size`, `is_tty`, raw-mode `SetConsoleMode` transitions, virtual-terminal enable. Same evidence as above: without console handles there is no handle to hand to these calls. Un-gating it would produce tests that assert nothing about the console. |
| `tui/build.rs`, `tui/src/terminal_posix.rs`, `tui/src/components/mod.rs`, `tui/src/rpc/mod.rs`, `cli/build.rs`, `cli/src/help.rs`, `cli/src/types.rs`, `cli/src/version.rs`, `cli/src/commands/mod.rs`, `cli/src/utils/mod.rs`, `cli/src/browser/chromium/mod.rs`, `cli/src/browser/safari/mod.rs` | absent from the report | `platform-unmeasured` | — | 12 source files are absent from `coverage/tui-report.json` (`verify.py blindspot` lists them). Two reasons: (a) **platform-gated** — `terminal_posix.rs` (926 lines, 33 fns) is `#[cfg(unix)]` and is measured by the POSIX run, not here; (b) **no instrumented translation unit** — `build.rs` runs in cargo's build-script process (`--include-build-script` would measure it), and `help.rs` (205 lines of help text), `types.rs`, `version.rs` and the `mod.rs` files declare no functions at all (verified: 0 `fn` per file), so llvm-cov gives them neither numerator nor denominator. A Windows "100 %" can therefore never prove anything about (a). |

### 6.2 Open gaps — no waiver claimed (full census, 55 files)

Bucket legend: **A** = error/edge arms of functions whose main body is covered;
**B** = a whole function/feature has no test yet; **C** = I/O, process or network arm
(needs an injected failure or a mock server); **D** = host console/platform path that the
test process cannot reach; **E** = line in the in-file test module (double/helper/test
body) that is counted because it ships in the same translation unit.

| file | uncovered | bucket / next step |
|---|---|---|
| `tui/src/app.rs` | 191 | E/B — the in-file `FakeTerminal` trait forwarders (lines ~117-147: `write`, `columns`, `start`, `stop`, `drain_input`, …) that no test calls, plus a handful of production arms. Needs a range-by-range audit before anything is claimed; a waiver here would be premature. |
| `tui/src/index.rs` | 111 | B/A — the CLI argument parser's arms (`--no-prompt-templates`, `--offline`, `--no-skills`, …) and the print-mode branches that call `execute_unary(...)`; several single-line entries are the closing braces of *covered* arms (span artifacts). Parser arms are cheap table-driven tests; the print-mode lines need the existing mock agent. |
| `tui/src/components/autocomplete.rs` | 63 | A — provider edge arms: `regex` capture misses (`caps.get(1)?`), directory-read failures, empty-candidate paths. |
| `tui/src/rpc/grpc_client.rs` | 37 | C/A — `suggest_skill` and transport-error arms. |
| `tui/src/clipboard.rs` | 33 | C — real clipboard tool probing and its failure arms (`osascript`/`xclip` absence is already faked; the probe-failure arms are not). |
| `tui/src/skills_cli.rs` | 24 | A/C — runner-error arms around the catalogue/skill process calls. |
| `tui/src/agent_supervisor.rs` | 19 | C — sidecar spawn: executable lookup failure, candidate loop, `child.kill()`/timeout; needs a fake program instead of the real binary. |
| `tui/src/paste.rs` | 11 | A/C — probe/decoder edge arms. |
| `tui/src/components/chat_area.rs` | 6 | A — folding/width edge arms. |
| `tui/src/paste_burst.rs` | 5 | A — burst-window edges. |
| `tui/src/tui.rs` | 5 | A — lifecycle arms. |
| `tui/src/components/keymap_view.rs` | 4 | A — highlight-walker arms. |
| `tui/src/external_editor.rs` | 4 | A — editor-resolution arms. |
| `tui/src/help_screen.rs` | 2 | A — rendering edges. |
| `tui/src/stdin_buffer.rs` | 2 | A — buffering edges. |
| `tui/src/update.rs` | 3 | A — version-comparison edges. |
| `tui/src/components/footer.rs` | 1 | A — single uncovered line (cheap). |
| `tui/src/components/input.rs` | 1 | A — single uncovered line (cheap). |
| `tui/src/components/markdown.rs` | 1 | A — single uncovered line (cheap). |
| `tui/src/components/menu.rs` | 1 | A — single uncovered line (cheap). |
| `tui/src/components/pager.rs` | 1 | A — single uncovered line (cheap). |
| `tui/src/components/usage_view.rs` | 1 | A — single uncovered line (cheap). |
| `tui/src/components/worktree_view.rs` | 1 | A — single uncovered line (cheap). |
| `tui/src/insert_history.rs` | 1 | A — scroll edge. |
| `tui/src/keybindings.rs` | 1 | A — binding parse edge. |
| `tui/src/skill_reco.rs` | 1 | A — recommendation edge. |
| `tui/src/terminal_image.rs` | 1 | A — image-chunk edge. |
| `tui/src/themes.rs` | 1 | A — theme lookup edge. |
| `tui/src/worktree.rs` | 1 | A — single uncovered line (cheap). |
| `cli/src/commands/skills.rs` | 336 | B — the whole skills CLI surface (`--json` variants, subcommands); the largest single gap in either crate and the first thing a successor should take. |
| `cli/src/commands/auth.rs` | 199 | B/C — the device-code login flow (`post`, `DeviceCodeResponse`, `verification_uri_complete` fallback); needs an HTTP mock, which `cli/src/test_server.rs` already provides. |
| `cli/src/commands/browser_tools.rs` | 114 | B/C — browser start/stop and the Safari/Chromium split. |
| `cli/src/commands/configure.rs` | 78 | B/E — the interactive prompter loop plus the non-injectable `StdioPrompter`/`configure()` wrapper that reads stdin. |
| `cli/src/commands/init.rs` | 52 | A/E — `init_command` wrapper and option defaults. |
| `cli/src/commands/doctor.rs` | 44 | A — probe `path`/`version` lines and status arms. |
| `cli/src/browser/safari/safari_manager.rs` | 40 | D/C — macOS WebDriver/Safari paths (compiled here, unreachable on this host). |
| `cli/src/browser/windows_process.rs` | 27 | C — Windows process enumeration failure arms. |
| `cli/src/commands/session_history.rs` | 26 | A/B — cursor/paging arms. |
| `cli/src/test_server.rs` | 12 | E/A — the mock server's own arms (they run only when a test drives them). |
| `cli/src/lib.rs` | 4 | A/E. |
| `cli/src/browser/discovery.rs` | 3 | C — host discovery arms (the two weak-spots of §5 are now asserted; these are the error arms). |
| `cli/src/commands/account.rs` | 3 | A — account command edges. |
| `cli/src/commands/session.rs` | 3 | A — session command edges. |
| `cli/src/browser/chromium/chromium_session.rs` | 3 | C — CDP session error arms. |
| `cli/src/browser/browser_state.rs` | 2 | C — state-file error arms. |
| `cli/src/browser/chromium/chromium_endpoint.rs` | 2 | C — endpoint discovery errors. |
| `cli/src/commands/session_compact.rs` | 2 | A. |
| `cli/src/utils/files.rs` | 2 | C — file-stat error arms. |
| `cli/src/browser/chromium/chromium_manager.rs` | 3 | C — launcher error arm; two of the three are the `None` arm of the host-dependent discovery test of §5 (this host *has* a browser, so the miss arm does not run here) |
| `cli/src/browser/chromium/chromium_navigation.rs` | 1 | C. |
| `cli/src/browser/chromium/execution_context.rs` | 1 | C. |
| `cli/src/browser/safari/webdriver_client.rs` | 1 | C. |
| `cli/src/commands/tools.rs` | 1 | A. |
| `cli/src/main.rs` | 1 | E — entry-point line. |
| `cli/src/test_env.rs` | 1 | E — test-support line. |
| `cli/src/utils/process.rs` | 1 | C. |

## 7. Findings for the reviewer and the next segment

1. **The detector in `verify.py weak` is approximate** — it misses `.expect()`,
   `.unwrap()`, `#[should_panic]` and assertion *helpers*, and its brace counter is
   thrown off by `{`/`}` inside string literals, which truncates a body and can hide a
   real assertion (`skills_cli::parse_catalogue_rejects_output_without_a_json_object`,
   §5). Its 25 offenders here contain 10 real tests. The audit above is the
   hand-checked version.
2. **`cargo llvm-cov` has no `--target-dir`** in the pinned 0.9.1; use
   `CARGO_TARGET_DIR`, and note it appends `llvm-cov-target`. A single-package run
   cleans the profile dir, so a two-crate report must be produced by **one** invocation
   (or the gate fails with "does not cover future-cli").
3. **Windows loopback refusal costs ≈2.2 s** — any test that asserts a dead-client error
   message must wait on the message, not on a 375 ms quiesce. This is likely to bite
   other crates (`future-cli` has the same shape of assertions).
4. **The console-gated tests self-skip silently** (they print `[skip] …`, which
   `cargo test` captures on success), so a green local run overstates what ran for
   `tui/src/terminal.rs`. A `--nocapture` run, or a console-attached run, is needed to
   see them.
5. Two tests had to be *un-hard-coded* on Windows rather than fixed: the POSIX path
   fixtures in `paste::path_candidates_*` and the canonicalised (`\\?\`) worktree
   fixtures. Any other crate that hard-codes `/tmp/...` for `Path::is_absolute` or
   compares a `canonicalize`d path against git's output has the same latent failure.
6. **Real bug found, not fixed (out of this task's scope):** none. The red tests were
   all test-side or timing-side; the production paths involved (git porcelain parsing,
   clipboard path building, `/worktree` planning) behaved correctly for their platform.
7. **Measurement noise is small but real.** Between the pre-fmt and post-fmt runs of
   the same code, `cli/src/browser/chromium/cdp_connection.rs` moved 1 → 0 uncovered lines
   (a teardown line a timing-dependent path sometimes executes) and
   `cli/src/browser/chromium/chromium_manager.rs` moved 2 → 3 (the strengthened test's
   host-dependent arm: this host finds a browser, so the miss arm does not run). Both
   totals are quoted from the final run in `coverage/tui-report.json`; re-read the report
   rather than trusting these numbers after further edits.
8. Next useful check: re-run the gate after the successor takes
   `cli/src/commands/skills.rs` (336) + `cli/src/commands/auth.rs` (199) +
   `tui/src/index.rs` (111) + the ~20 single-line tui files; those four groups alone
   would move future-cli to ≈98% and future-tui past 98.5% and would clear 30 of the
   55 open-gap rows. `tui/src/terminal.rs` (388) needs either an injectable
   stdout/raw-mode seam (design work) or the console-attached/POSIX measurement that
   the waiver above depends on.
