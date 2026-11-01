# tauri (`desktop/src-tauri`) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The llvm-cov
summary's `Lines` column is not the acceptance number.

**Lane** — `desktop/src-tauri` (crate `futureos`). Two legs, both `--features test-support`:
`cargo llvm-cov -p futureos --lib` (the default `gui` feature) and the `harness = false`
main-thread driver `cargo llvm-cov -p futureos --test gui_main_thread_drive`. The lib is a
single target, so this is the whole crate's unit-test surface — there are no integration
binaries beyond the driver.

**Last measurement**: `test/tauri-cov2` (waves 6-11), rustc **1.99.1** (`rust-toolchain.toml`),
`ulimit -n 8192`, raw profiles deleted first (`--no-report` accumulates). Artifacts
(gitignored) in `coverage/2026-11/tauri/`: `wave13-lcov.info` (the numbers quoted below; wave 13
re-measures wave 11's tests on the base that landed under them, `d3b2c58fd`),
`wave13-lib.log` / `wave13-driver.log` / `wave13-report.log`, the run script `wave13.sh`, the
per-wave records `wave6.md` (waves 6-7), `wave9.md` (waves 8-9) and `wave11.md` (waves 10-11), the
classification `classification.md`, and the diff tools `diff_lane.py` / `da0_lane.py` /
`da0_lines.py` / `da0_file.py`.

`da0_file.py` takes **one** file by its whole record path; `da0_lines.py` matches a suffix, which
silently unions `desktop/src-tauri/src/lib.rs` with `host/src/lib.rs` and `packages/*/src/lib.rs`
(546 lines against the real 48). Quote `da0_file.py` for a per-file row.

```bash
# One measurement, both legs, fresh profiles (the script does the rm for you).
coverage/2026-11/tauri/wave13.sh
python3 coverage/2026-11/tauri/da0_lane.py  coverage/2026-11/tauri/wave13-lcov.info           # lane total
python3 coverage/2026-11/tauri/da0_file.py  coverage/2026-11/tauri/wave13-lcov.info \
        desktop/src-tauri/src/lib.rs                                                         # per line, one file
cargo test -p futureos --lib                                  # gate: 423 passed, ulimit -n 8192
cargo test -p futureos --test gui_main_thread_drive --features test-support
```

## 1. Where the lane stands

| | `DA:0` | note |
|---|---:|---|
| baseline (wave 1, `05a260c6f`) | **1427** | `coverage/2026-11/rust/tauri-lcov.info` |
| wave 5 (`963`) | 963 | preparation chain moved into the driver |
| wave 6 | **864** | the real update worker end to end (−99) |
| wave 7 | **819** | worker failure arms, `confirm`, `update_process` refusals (−45) |
| wave 9 (first measurement) | 639 | the transfer commands, the host-event sink, the two open update items (−180), on the pre-`a8669c8b3` base |
| wave 10 | **638** | the same tests after `a8669c8b3` landed under them; per-file identical except the interleaving line below |
| wave 11 | **635** | one arm of each update wrapper that a mock can reach; the rest of `update.rs` classified per cluster in §2.8 (the first round that measured its own unreachability instead of asserting it) |

The lane is **not** at per-line 0 and does not claim to be. What it does claim: every remaining
line in the files this table covers is either registered below or named as a work item in §4;
nothing in §2 is claimed as "covered".

| file | `DA:0` | disposition |
|---|---:|---|
| `src/commands/update.rs` | 191 | §2.8 — every cluster classified; 129 of them are a **product decision** (§2.8.1) |
| `src/agent_supervisor.rs` | 88 | work items — the watchdog loop body and the quit dialogs |
| `src/lib.rs` | 47 | §2.6 registered (real handle / mock-always-Ok / startup) + work items |
| `src/terminal/server.rs` | 56 | known ±1 fluctuation (§2.5) + interleaving-dependent test arms |
| `src/main.rs`, `src/terminal/pty.rs`, … | 429 | work items (see `classification.md` for the wave-1 bucket split) |
| `src/commands/history_transfer.rs` | 2 | 1 measured-unreachable command arm (§2.7) + 1 scaffolding arm |
| `src/commands/update_journal.rs` | 12 | 2 real items (§4.2) + attribution (§2.4) |
| `src/commands/update_process.rs` | **0** | closed in wave 9 (§4.1) |
| `src/commands/update_macos.rs` | 5 | 3 registered (§2.1) + 2 attribution (§2.4) |
| `src/commands/update_adapters.rs` | 15 | platform facades the worker leg reaches only on another OS |

## 2. Registered waivers

Codes used by this lane (the other lanes' `W*` codes are listed in their own files):

- `T1` — **production-only entry**: the unit derives its input from `current_exe()` being inside
  a built `.app`. A test process is never that, and no seam may change the production path to
  make it so.
- `T4` — **attribution artifact**: the statement demonstrably ran (its neighbours read `DA>0`,
  and its observable effect is asserted elsewhere), but this line's own record stays `0`.
- `T5` — test code whose body only runs when an assertion fails.
- `T6` — **environment-bound arm**: the error the line handles cannot be produced in a test
  process on the platforms CI runs (the syscall returns a *different* errno, or the mocks
  always succeed). Coupled with a positive control stating what *is* produced.

### 2.1 `src/commands/update_macos.rs` — `T1`

| line | code | mechanism (citation) | positive control | pinned by |
|---|---|---|---|---|
| 86-88 | `T1` | `Prepared::new` → `Self::prepare_at(bytes, operation, bundle()?)`, and `bundle()` is `tauri_plugin_updater::extract_path_from_executable(&current_exe())` (`update_macos.rs:11-17`): from a test binary it refuses, which is why the chain behind it takes the path as a parameter (`prepare_at`, `update_macos.rs:74-80`) | run the driver from a staged `.app` whose `current_exe()` is inside it — every line behind `prepare_at` then runs, and already does in the libtests | `prepared_case`-based libtests (`a_payload_must_hold_exactly_one_application_bundle` …) cover `prepare_at`; `production_hands_the_prepared_bundle_to_this_executable` pins the only thing `new` adds, `bundle()` |
| 187 | `T1` | `None => update_helper_program()?` — the production hand-off target; every test path passes `Some(helper)` (`prepare_with`, `update_macos.rs:118-146`), and the `None` caller is `Prepared::new` (line 86) | same `.app` host as above | `production_hands_the_prepared_bundle_to_this_executable`; deliberately **not** written as `helper.map_or_else(...)`, which would mark the line covered while the production arm never ran (wave5 §5.1) |
| 376-378 | `T1` | `confirm(id)` = `confirm_at(&bundle()?, id)`: the production wrapper, whose path again comes from `current_exe()` | a confirmed installation inside a real `.app` | `confirm_at` itself is covered by the driver's confirm step (10/10 check "a confirmed installation removes the staging directory it kept for rollback"); the wrapper adds only `bundle()` |

### 2.4 `src/commands/update_macos.rs` — `T4`

| line | code | mechanism | positive control | pinned by |
|---|---|---|---|---|
| 285 | `T4` | the `)?);` of `drop(Prepared::prepare_with_helper(...)?)` in `prepare_and_drop` — llvm attributes the multi-line call's closing line to the `?` *error* path, which cannot run on success | make `prepare_with_helper` fail after the operation begins | `prepare_and_cancel_update_for_tests` ran green in the driver (wave5 §4.2: "a prepared update that is cancelled leaves no staging directory either") — the call executed, its effect is asserted |
| 387 | `T4` | the closing brace of `confirm_at` | — | same as above: the driver's confirm step fails if this body does not run |

### 2.5 `src/terminal/server.rs` — the known ±1

`DA:0` here moves by one line between identical runs (56 ↔ 57). The two real-shell end-to-end
fixtures answer a shell's cursor query and read frames on an **interleaving**: which match arms
and which DSR branch execute depends on how the shell splits its output across reads. Measured
both directions (`wave5-lcov.info` 56 → `wave6-lcov.info` 57 → `wave7-lcov.info` 56) with no
change to the arms in between; wave5 §5.2 recorded the same family earlier.

This is **not** registered as a waiver — the lines are reachable — but a round that quotes this
file's number should quote the run it came from, not the field.

`src/lib.rs:808` belongs to the same family and was measured on both sides: the drain loop's
100 ms idle arm read `DA>0` in `wave8-lcov.info` (47 lines, no test had ever started the thread)
and `0` in `wave9-lcov.info` (48), because wave 9's event case is the first to start it. Nothing
in the arm changed — and `wave10-lcov.info` has it back at `DA>0` (47), which is what a ±1 looks
like.

### 2.6 `src/lib.rs` — `T1` / `T6`

`present_host_event` is the only sink `future_host::events::attach` registers, so the case list in
`gui::host_event_tests` covers all eleven variants and asserts the one that has a payload-free side
effect (the runtime revision only advances inside `emit_thread_runtime_updated`). The remaining
lines in the file:

| line | code | class | mechanism | positive control |
|---|---|---|---|---|
| 133 | `activate_main_window(app)` in the `ControlClaimed` arm | `T1` | reads the process-global `APP_HANDLE`, whose only writer is `run_with`'s setup; the handle has no reset, so a test may not set it | `activate_main_window` itself is covered both ways (`activate_main_window_restores_shows_and_focuses_the_main_window`, `…tolerates_a_missing_main_window`) |
| 526, 817 | `handle.emit(…)` inside the `APP_HANDLE` arms | `T1` | same global | `emit_tasks_updated` / `emit_runtime_updates_on` bodies are covered through their `_on(handle, …)` siblings; `the_emit_entry_points_are_noops_before_a_window_exists` pins the `None` half |
| 168, 171, 200 | `unminimize` / `show` / `set_focus` error arms | `T6` | the mock runtime answers `Ok(())` to all three (`tauri-2.12.5/src/test/mock_runtime.rs:695-707`) | `activate_main_window_restores_shows_and_focuses_the_main_window` runs the `Ok` half on the same window |
| 892-948 (the menu handler's body) | `activate_main_window`, the `open-settings` / `menu-command` emits, the webview reload | `T1` | a `Builder::on_menu_event` listener is dispatched only from `RuntimeRunEvent::UserEvent(EventLoopMessage::MenuEvent)` (`tauri-2.12.5/src/app.rs:2755-2771`), which `muda` posts through the event-loop proxy (`app.rs:2520-2522`). The mock runtime's `EventProxy::send_event` returns `Ok(())` and **drops** the event (`src/test/mock_runtime.rs:1158-1165`), so no menu event can reach a listener on a mock app — measured, not inferred from "needs a real handle" | `gui_builder::<MockRuntime>()` is built by `quit_admission_closed_before_the_ready_event_withholds_the_window_work` (`lib.rs:2787`), so the *registration* is covered; only these bodies are not |
| 933, 937, 939-948 (the window handler's body) | the `main`-label guard and the two `QuitDecision` arms | `T1` | `Builder::on_window_event` handlers live in the window manager and are invoked from the per-window `on_window_event` closure (`tauri-2.12.5/src/manager/window.rs:98-110`); the mock runtime **discards** that closure — `MockWindowDispatcher::on_window_event` returns a fresh id without storing `f` (`src/test/mock_runtime.rs:722-723`). The mock's `close()` delivers `RunEvent::WindowEvent { CloseRequested }` to the `app.run` callback instead (`mock_runtime.rs:1360-1382`), which is a different seam than the builder's handler | `agent_supervisor::on_close_requested` and both arms it answers are covered by their own cases; only the routing from a window event is not |
| 863-865, 972-1000, 1249, 1260, 1293-1294 | `run()` and `run_with()`'s startup and setup-failure paths | `T1` | need the real `Wry` event loop; `run_with` also writes the process-global `APP_HANDLE` | — |
| 1751, 2827 | test bodies | `T5` | their own `return` / `panic!` arms | — |

### 2.7 `src/commands/history_transfer.rs:69` — `T1`-adjacent, **measured**

`begin_session_history_transfer`'s `#[tauri::command]` line carries the async wrapper's
argument-deserialization error arm. It was measured rather than assumed: the command's arguments
are `WebviewWindow`, `thread_id: String`, `before: Option<i64>`, `limit: i64`,
`history_source: Option<String>`, and
`the_transfer_command_refuses_a_raw_bytes_payload` sends an `InvokeBody::Raw` request precisely
because `CommandItem::deserialize_option` rejects a raw payload
(`tauri-2.12.5/src/ipc/command.rs:139-151`). The request *is* refused — the assertion pins the
`bytes payload` message so a "command not found" could not pass for it — and the record lands on
the **signature** line: the first JSON argument (`threadId`) fails first. The attribute line can
therefore only be reached with a body that satisfies `threadId`, `before` and `limit` and still
fails, and no such body exists (`Option` accepts a missing key; a raw payload fails one argument
earlier). Same family as the `bootstall_builtin_skills` note in `commands/skills.rs`.

### 2.8 `src/commands/update.rs` — 191 lines, classified per cluster

Until wave 11 this file's row said "the IPC wrappers this lane has not reached yet". That is no
longer an adequate description: wave 11 measured each boundary, and every line below is now either
registered here with the mechanism that keeps it out of a test process, or filed as a **product
decision** (§2.8.1) with the seam that would make it observable. The `DA:0` set is 37 runs; the
groups below sum to 191.

| lines | n | class | mechanism | positive control |
|---|---:|---|---|---|
| 157-161 | 5 | `C` | `confirm_update_startup` is `#[cfg(not(test))]` **and** `pub(crate)`. The lib leg compiles it out, and the driver test crate can only name `pub` items, so neither leg can call it. Its only caller is `run_with`'s startup | the journal half it wraps (`journal::confirm_startup`) has its own cases; this function adds only the report on error |
| 454-457, 460-461 | 6 | `T1`-adjacent | `check_app_update`'s wrapper and body. Both routes are refused: the product's `generate_handler!` list is built on a `Wry` builder inside `run_with` (`lib.rs:1030`), and any invocation resolves its own manifest URL from the build channel — **measured**: dispatching it from a mock IPC fixture answered with a real `0.0.2-104+nightly` manifest fetched from `dl.future-os.cn` | `perform_app_update_check` (same body, endpoint injected) is covered by `the_scheduled_check_body_passes_its_endpoint_through` |
| 479 | 1 | platform-unmeasured | the `!platform_allows_updates()` return. This host's build answers `true` — measured, because line 482 (the `Dev`/`Local` branch behind it) is `DA>0` in the same run. Reaching 479 needs a platform where the check fails: a Windows portable ZIP (`bundle_type() == None`) or an unwritable bundle (`access(parent, W_OK)`, `update_macos.rs:27-38`) | `check_manual_update`, which it returns, is covered directly |
| 760-763 | 4 | `T5` | the install closure of `cleanup_failure_never_enters_installation`. It running **is** the failure that case asserts | `installation_failure_requires_process_recovery_after_successful_cleanup` runs the sibling closure |
| 1325, 1441, 1456 | 3 | `T5` | `panic!` arms of three restart cases | each case passes on its `Ok` half |
| 1490-1492 | 3 | platform-unmeasured | `relaunched.set(true)` for a `failed-restored` record. macOS reaches the block only through `recovery::ProgramFingerprint::verify_saved` (1977), which refuses — **measured**: 1977 is `DA>0` in this run, so the arm runs and the relaunch does not | `a_failed_restored_record_requires_platform_recovery_verification` asserts both iterations; Windows and Linux reach 1490-1492 |
| 1633 | 1 | `T4`-adjacent | `install_app_update`'s `#[tauri::command]` attribute. Same family as §2.7, and now measured for this command: a raw-bytes request is refused on the **signature** line ("expected a value for key `language` … the IPC call used a bytes payload"), and a dispatch that reaches the body leaves the attribute at `0` | `the_install_command_refuses_a_manual_only_build_over_ipc` reaches the body over IPC with `{}` — the shape the frontend sends — and gets the wrapper's own refusal |
| 1689, 2005 | 2 | `T4` | closing braces of `install_app_update_impl` and `restart_after_app_update_with`. Every statement inside both blocks is `DA>0` in the same run, so the record is the block end, not an arm | — |
| 1716-1723, 1726-1727, 1729-1751 | 33 | `T1` | `acknowledge_update_repair` takes `app: tauri::AppHandle`, i.e. `AppHandle<Wry>` — the runtime parameter is not generic, so there is nothing to substitute, and `tauri::test` only ever builds `AppHandle<MockRuntime>`. The body then opens a blocking native dialog | `journal::acknowledge_repair`, the only thing the body changes, is covered by the `update_journal.rs` cases |
| 1973, 1975-1977 | 4 | `T1` | `restart_after_app_update`: same concrete handle, and its body calls `app.restart()`, which re-execs the process | the injectable body `restart_after_app_update_with` has four cases |
| 1802-1867, 1872-1962, 1968 | 129 | **product decision** | §2.8.1 | — |

#### 2.8.1 The 129 lines behind `download` — a product decision, not a test-writing problem

`install_app_update_inner` is covered down to its `.map_err("Failed to download and verify the
update")` (1801) and not one line past it. That boundary is not arbitrary; it is where three
things this crate cannot inject begin:

1. **`update.download()` verifies a minisign signature.** `mock_app_with_updater` pins
   `pubkey: "dummy"`, and `minisign-verify` is verify-only — there is no signing crate anywhere in
   `Cargo.lock`, so no fixture can produce a payload that passes. The closest existing arm,
   `install_app_update_downloads_and_fails_signature_verification`, stops exactly at 1801.
2. **Past `prepare`, the flow blocks on a native dialog.** `confirmation_app.dialog()…
   .blocking_show()` (1839-1848) needs the dialog plugin's real native handle; the mock runtime's
   sentinel is precisely what `agent_supervisor::prepare_quit_confirmation` keeps out of `rfd`
   (`lib.rs:2803-2805`). The result is a hang, not a failure.
3. **Past the dialog, `adapters::commit` performs a real platform installation** of the running
   bundle — a test binary is not inside one (`T1`), and driving it from one would be destructive.

Reaching any of it therefore needs production seams, and this campaign's rule is that a lane files
the decision rather than changing the product to raise a number:

| seam | would cover | shape |
|---|---|---|
| a `download` injection point beside the existing `prepare_with_helper` / `confirm_at` | 1802-1817 (+1816-1817), 1820-1835 | the idiom this lane already uses: `#[cfg(any(test, feature = "test-support"))]` facade, production path unchanged |
| a `confirm` answer beside it | 1837-1867 incl. the cancel arm | same |
| a `commit` injection point | 1872-1956, 1957-1962, 1968 | same, and the riskiest of the three |

Until one exists these 129 lines are unreachable **by construction**, and no amount of test
writing moves them. That is the honest reading of a 191-line gap in a 2 000-line file whose second
half is glue between an updater plugin, a native dialog and a platform installer: the file's own
unit-test surface is already 1 226 executable lines, and 33 of the remaining 191 are a single
command that cannot be handed a mock handle.

## 3. Products of the coverage work (not waivers)

Two production seams were required and are gated, behaviour unchanged. Both are the same 1:1
split the coverage campaign approved for this lane (README §"已批准的三处 tauri 例外"):

| seam | why | gate |
|---|---|---|
| `probe` → `pub(super)` | its 10 s deadline can only be *expired* by a candidate that outlives it (+10 s, unaffordable beside 400 instrumented libtests) | `#[cfg(any(test, feature = "test-support"))]` facade in `commands::update` |
| `Prepared::prepare_with_helper` / `confirm_at` | the chain and the confirmation derive their bundle path from `current_exe()`; injecting it is what makes every step behind them observable | same |

## 4. Work items

### 4.1 `update_process.rs:288-289` — **closed in wave 9**

`agent_lease`'s "another Agent started before installation" arm: `try_lock_exclusive()` fails
because a **live holder** exists. **Closed in wave 9** —
`the_agent_lease_refuses_while_another_agent_holds_the_instance_lock`. No child process is
needed and no dependency had to move: `flock(2)` locks belong to the open file description, so
two descriptors on one file *in one process* are two holders, and `fs2` is already a normal
dependency of this crate (a test target sees those too — the note that it was missing from
`[dev-dependencies]` was wrong, and adding it would have been redundant). The same call succeeds
once the holder unlocks, which is what makes the arm the lock rather than the fixture.

### 4.2 `update_journal.rs` — 12 lines left, 5 of them attribution

Closed in wave 9: the `latest()` permission arm (196-200) via an operation directory that cannot
be written into; its sibling `Err(..)` arm (202) via an `observer.lock` that is a **directory**
(`EISDIR`, not `PermissionDenied`); the "interrupted after shutdown began" branch (212-215) via a
seeded record at stage `installing`; `confirm_startup`'s lease refusal (233) with the transaction
lock held by the case, its observer refusal (252-253) with `observer.lock` held, and its body
(237-266) through to `installed`.

What is left:

| line | class | mechanism |
|---|---|---|
| 170-173 | `T6` | `acquire_transaction_lease`'s **non-`WouldBlock`** `flock` error arm. `transaction_lock()` opens the file read+write (so no `EBADF`), on a regular filesystem (so no `ENOLCK`/`EOPNOTSUPP`); the mock-free way to reach it does not exist on the CI filesystems. Positive control: the `WouldBlock` arm on the same call is covered by every `Operation::begin` collision test |
| 249-250 | `T6` | `raw_os_error() == Some(32)` in `confirm_startup`: Windows' `ERROR_SHARING_VIOLATION`. On Unix errno 32 is `EPIPE`, which `open` never returns. Positive control: the other two arms of the same `match` run in `latest_hands_back_an_undecided_record…` and `latest_refuses_an_observer_lock…` |
| 217, 261-263, 266 | `T4` | closing braces and the multi-line `host_environment::report(&format!(…))` in the cleanup-failed branch: line 260 (`DA=1`) is the statement, 261-263 are its continuation |
| 403 | `T4` | the `)?;` of `acknowledge_repair`'s `OpenOptions::open(…)`, attributed to the error path of a call that succeeded |

### 4.3 The rest of the lane

`update.rs` (191) is now classified cluster by cluster in §2.8, and 129 of it is filed as a product
decision there rather than work for a later round — the largest single finding of wave 11 is that
this file has no reachable lines left that a test can take without a production seam.
`agent_supervisor.rs` (88) is next by size, then the startup paths in `lib.rs` (§2.6).
`history_transfer.rs` (2) and `update_process.rs` (0) are closed. See `classification.md` for
the wave-1 class split (R reachable / E error arms / P platform-bound / C `cfg(not(test))` /
A attribution / T test code) over the original 1427.

## 5. How the wave 6-11 numbers were produced and falsified

Every scenario added in these rounds was made red before it was trusted, and each failure named
the derived value it was built on (`coverage/2026-11/tauri/wave6.md` §4,
`coverage/2026-11/tauri/wave9.md` §4):

| scenario | falsification | result |
|---|---|---|
| worker install (driver) | worker dispatch disabled | `0/6`, first line is the child's own marker |
| probe timeout | fast candidate | `after 0.2s` instead of 10 s |
| persist failure | directory left readable | `ok: true` from a worker that succeeded |
| agent-lease failure | `agent` made a directory | `ok: true`, journal `pending-validation` |
| `confirm` | staging kept | "staging left behind" |
| Agent lease held (wave 9) | `try_lock_exclusive` removed | the live-holder case goes red |
| journal observer/confirm arms (wave 9) | each guard broken in turn (5 mutations) | each case goes red |
| transfer commands (wave 9) | owner check dropped, error code renamed, publish skipped | each case goes red |
| host-event routing (wave 9) | the runtime-update arm dropped | the revision stops advancing |
| install refuses a stopping Desktop (wave 11) | `is_stopping() && false` | the call falls through to `app.updater_builder()`, which with a plugin-less mock app **panics** (`state() called before manage() for tauri_plugin_updater::UpdaterState`) — the guard is also what keeps this fixture off the updater |
| the install flag comes back on a refusal (wave 11) | `std::mem::forget(install_guard)` on the return path | the flag is still claimed, so the follow-up `compare_exchange` fails |
| the wrapper reaches its own guard over IPC (wave 11) | `automatic_install_supported(channel)` → `true` | same updater panic instead of the manual-only refusal |

One check had to be rewritten because it passed for the wrong reason: the persist-failure
scenario first failed on a *missing record*, not on the directory permission (wave6.md §4.3).
Wave 9's "another window cannot read it" case was the same shape — both mock windows were labelled
`main`, so the ownership assertion could never fail — and went red as soon as the second window
was given its own label (wave9.md §4).
